//! Servidor na conta Cloudflare do usuário pela API REST (sem wrangler nem Node): o assistente
//! "Criar meu servidor" e o "Atualizar servidor" do app. Plano em
//! `docs/PLANO-onboarding-servidor.md` (Fase 4).
//!
//! O Worker vem pronto em cada release do app (`server.json`, gerado por
//! `server/scripts/bundle.mjs`): módulo empacotado, migrações, assets e o que o `wrangler.jsonc`
//! configura. Aqui se repete o que o `wrangler deploy` faz: D1 + migrações (tabela
//! `d1_migrations`, a mesma do wrangler), bucket R2, upload dos assets, do Worker com os bindings,
//! cron e subdomínio `workers.dev`.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use ureq::http::Response;
use ureq::{Agent, Body};

use crate::api::{Client, CloudError, API_VERSION};
use crate::login::random_b64;

const API: &str = "https://api.cloudflare.com/client/v4";

/// Criação de token já com as permissões (Workers, D1, R2 e leitura da conta).
pub const TOKEN_URL: &str = "https://dash.cloudflare.com/profile/api-tokens?permissionGroupKeys=%5B%7B%22key%22%3A%22workers_scripts%22%2C%22type%22%3A%22edit%22%7D%2C%7B%22key%22%3A%22d1%22%2C%22type%22%3A%22edit%22%7D%2C%7B%22key%22%3A%22workers_r2%22%2C%22type%22%3A%22edit%22%7D%2C%7B%22key%22%3A%22account_settings%22%2C%22type%22%3A%22read%22%7D%5D&accountId=*&zoneId=all&name=Glint%20server";

/// Página do R2 (ativar exige cartão cadastrado, mesmo no plano grátis).
pub fn r2_url(account: &str) -> String {
    format!("https://dash.cloudflare.com/{account}/r2/overview")
}

/// Segredo aleatório (`SETUP_TOKEN`, `SIGNING_KEY`).
pub fn random_secret() -> String {
    random_b64(32)
}

#[derive(Debug, thiserror::Error)]
pub enum CfError {
    #[error("token da Cloudflare inválido ou sem as permissões")]
    BadToken,
    #[error("o token não dá acesso a nenhuma conta")]
    NoAccount,
    /// R2 ainda não ativado na conta (pede cartão no painel).
    #[error("R2 não está ativado na conta")]
    R2Disabled { account: String },
    #[error("já existe um servidor nesta conta")]
    Exists,
    /// Atualizar só funciona no endereço `<worker>.<conta>.workers.dev`.
    #[error("o servidor não está num endereço workers.dev")]
    NotWorkersDev,
    #[error("o servidor não foi encontrado nesta conta")]
    NotFound,
    #[error("pacote do servidor {0} não encontrado")]
    NoBundle(String),
    #[error("o servidor não respondeu a tempo")]
    Timeout,
    #[error("Cloudflare: {0}")]
    Api(String),
    #[error(transparent)]
    Cloud(#[from] CloudError),
}

impl From<ureq::Error> for CfError {
    fn from(e: ureq::Error) -> Self {
        CfError::Cloud(e.into())
    }
}

type Result<T> = std::result::Result<T, CfError>;

#[derive(Deserialize)]
pub struct Module {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub b64: String,
}

#[derive(Deserialize)]
pub struct Migration {
    pub name: String,
    pub sql: String,
}

#[derive(Deserialize)]
pub struct Asset {
    pub path: String,
    pub hash: String,
    pub size: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub b64: String,
}

#[derive(Deserialize)]
pub struct Names {
    pub worker: String,
    pub database: String,
    pub bucket: String,
}

/// `server.json` de um release.
#[derive(Deserialize)]
pub struct Bundle {
    pub version: String,
    pub compatibility_date: String,
    pub main_module: String,
    pub modules: Vec<Module>,
    pub migrations: Vec<Migration>,
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub headers: Option<String>,
    #[serde(default)]
    pub crons: Vec<String>,
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    pub names: Names,
}

/// Baixa o pacote do servidor da versão `version` do app (release `v<version>` de `repo`).
pub fn fetch_bundle(repo: &str, version: &str) -> Result<Bundle> {
    let url = format!("https://github.com/{repo}/releases/download/v{version}/server.json");
    let mut resp = agent(120).get(&url).call()?;
    match resp.status().as_u16() {
        404 => return Err(CfError::NoBundle(version.into())),
        s if s >= 400 => return Err(CfError::Api(format!("GitHub respondeu {s}"))),
        _ => {}
    }
    resp.body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_json()
        .map_err(|e| CfError::Api(format!("server.json inválido: {e}")))
}

/// Etapas (para a barra de progresso).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Download,
    Account,
    Database,
    Storage,
    Upload,
    Publish,
    Waiting,
    Admin,
}

/// Servidor novo.
pub struct NewServer {
    /// `SERVER_NAME` (vazio = nome do app).
    pub name: String,
    /// `SERVER_LANG`: `en` ou `pt-BR`.
    pub lang: String,
    /// Segredo com que o app cria o admin (`POST /api/v1/setup`).
    pub setup_token: String,
    /// Retomar uma criação que já subiu o Worker com este `setup_token` (o Worker existente
    /// não é recusado e os segredos dele ficam).
    pub resume: bool,
}

fn agent(timeout_s: u64) -> Agent {
    Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(timeout_s)))
        .build()
        .into()
}

#[derive(Deserialize)]
struct Msg {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    errors: Vec<Msg>,
    #[serde(default)]
    result: Value,
}

/// Chamadas da API da Cloudflare com o token do usuário, já numa conta.
struct Cf {
    agent: Agent,
    token: String,
    account: String,
}

/// Código de erro "não existe" (script, banco, bucket, subdomínio).
const NOT_FOUND_CODES: [i64; 2] = [10006, 10007];
const R2_DISABLED: i64 = 10042;

impl Cf {
    fn url(&self, path: &str) -> String {
        format!("{API}/accounts/{}{path}", self.account)
    }

    /// Resposta → `result`; `Ok(None)` quando não existe (404).
    fn read<T: DeserializeOwned>(&self, mut resp: Response<Body>) -> Result<Option<T>> {
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let env: Envelope = serde_json::from_str(&text).map_err(|_| {
            CfError::Api(format!(
                "resposta inesperada ({status}): {}",
                text.chars().take(200).collect::<String>()
            ))
        })?;
        if status < 400 && env.success {
            return serde_json::from_value(env.result)
                .map(Some)
                .map_err(|e| CfError::Api(format!("resposta inválida: {e}")));
        }
        if env.errors.iter().any(|e| e.code == R2_DISABLED) {
            return Err(CfError::R2Disabled {
                account: self.account.clone(),
            });
        }
        if status == 404 || env.errors.iter().any(|e| NOT_FOUND_CODES.contains(&e.code)) {
            return Ok(None);
        }
        if status == 401
            || status == 403
            || env.errors.iter().any(|e| matches!(e.code, 9109 | 10000))
        {
            return Err(CfError::BadToken);
        }
        let msg = env
            .errors
            .iter()
            .map(|e| format!("{} ({})", e.message, e.code))
            .collect::<Vec<_>>()
            .join("; ");
        Err(CfError::Api(if msg.is_empty() {
            format!("HTTP {status}")
        } else {
            msg
        }))
    }

    fn must<T>(v: Option<T>, what: &str) -> Result<T> {
        v.ok_or_else(|| CfError::Api(format!("{what}: não encontrado")))
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>> {
        let r = self
            .agent
            .get(self.url(path))
            .header("Authorization", format!("Bearer {}", self.token))
            .call()?;
        self.read(r)
    }

    fn send<T: DeserializeOwned>(&self, method: &str, path: &str, body: &Value) -> Result<T> {
        let url = self.url(path);
        let auth = format!("Bearer {}", self.token);
        let r = match method {
            "PUT" => self
                .agent
                .put(url)
                .header("Authorization", auth)
                .send_json(body)?,
            _ => self
                .agent
                .post(url)
                .header("Authorization", auth)
                .send_json(body)?,
        };
        Self::must(self.read(r)?, path)
    }

    fn d1(&self, db: &str, sql: &str) -> Result<Vec<Value>> {
        #[derive(Deserialize)]
        struct Res {
            #[serde(default)]
            results: Vec<Value>,
        }
        let r: Vec<Res> = self.send(
            "POST",
            &format!("/d1/database/{db}/query"),
            &json!({ "sql": sql }),
        )?;
        Ok(r.into_iter().flat_map(|r| r.results).collect())
    }
}

/// Primeira conta que o token alcança (o link do token já vem com "todas as contas").
fn accounts(token: &str) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Acc {
        id: String,
    }
    let cf = Cf {
        agent: agent(60),
        token: token.trim().to_string(),
        account: String::new(),
    };
    let r = cf
        .agent
        .get(format!("{API}/accounts?per_page=50"))
        .header("Authorization", format!("Bearer {}", cf.token))
        .call()?;
    let list: Vec<Acc> = cf.read(r)?.unwrap_or_default();
    Ok(list.into_iter().map(|a| a.id).collect())
}

fn connect(token: &str, account: String) -> Cf {
    Cf {
        agent: agent(300),
        token: token.trim().to_string(),
        account,
    }
}

/// Banco D1 pelo nome (cria se faltar) → id.
fn database(cf: &Cf, name: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Db {
        uuid: String,
        name: String,
    }
    let list: Vec<Db> = cf
        .get(&format!("/d1/database?name={name}"))?
        .unwrap_or_default();
    if let Some(db) = list.into_iter().find(|d| d.name == name) {
        return Ok(db.uuid);
    }
    let db: Db = cf.send("POST", "/d1/database", &json!({ "name": name }))?;
    Ok(db.uuid)
}

/// Migrações que faltam, como o `wrangler d1 migrations apply` (mesma tabela de controle).
fn migrate(cf: &Cf, db: &str, migrations: &[Migration]) -> Result<()> {
    cf.d1(
        db,
        "CREATE TABLE IF NOT EXISTS \"d1_migrations\"(\n\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\n\
         name TEXT UNIQUE,\n\
         applied_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL\n);",
    )?;
    let done: HashSet<String> = cf
        .d1(db, "SELECT name FROM \"d1_migrations\" ORDER BY id")?
        .into_iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str).map(str::to_string))
        .collect();
    for m in migrations.iter().filter(|m| !done.contains(&m.name)) {
        tracing::info!("migração {}", m.name);
        cf.d1(
            db,
            &format!(
                "{}\nINSERT INTO \"d1_migrations\" (name) values ('{}');",
                m.sql,
                m.name.replace('\'', "''")
            ),
        )?;
    }
    Ok(())
}

fn bucket(cf: &Cf, name: &str) -> Result<()> {
    if cf.get::<Value>(&format!("/r2/buckets/{name}"))?.is_none() {
        cf.send::<Value>("POST", "/r2/buckets", &json!({ "name": name }))?;
    }
    Ok(())
}

/// Corpo `multipart/form-data` montado à mão (o ureq não tem).
struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new() -> Self {
        Self {
            boundary: format!("glint-{}", random_b64(18).replace(['-', '_'], "x")),
            body: Vec::new(),
        }
    }

    fn part(&mut self, name: &str, filename: Option<&str>, ctype: &str, data: &[u8]) {
        let file = filename
            .map(|f| format!("; filename=\"{f}\""))
            .unwrap_or_default();
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"{file}\r\nContent-Type: {ctype}\r\n\r\n",
                self.boundary
            )
            .as_bytes(),
        );
        self.body.extend_from_slice(data);
        self.body.extend_from_slice(b"\r\n");
    }

    fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

/// Sobe os assets que a Cloudflare ainda não tem → JWT de conclusão.
fn upload_assets(cf: &Cf, worker: &str, b: &Bundle) -> Result<String> {
    #[derive(Deserialize)]
    struct Session {
        jwt: Option<String>,
        #[serde(default)]
        buckets: Vec<Vec<String>>,
    }
    #[derive(Deserialize, Default)]
    struct Done {
        jwt: Option<String>,
    }
    let manifest: serde_json::Map<String, Value> = b
        .assets
        .iter()
        .map(|a| (a.path.clone(), json!({ "hash": a.hash, "size": a.size })))
        .collect();
    let s: Session = cf.send(
        "POST",
        &format!("/workers/scripts/{worker}/assets-upload-session"),
        &json!({ "manifest": manifest }),
    )?;
    let mut jwt = s.jwt.clone().unwrap_or_default();
    for group in &s.buckets {
        let mut mp = Multipart::new();
        for h in group {
            let a = b
                .assets
                .iter()
                .find(|a| &a.hash == h)
                .ok_or_else(|| CfError::Api(format!("asset pedido não existe: {h}")))?;
            mp.part(h, Some(h), &a.kind, a.b64.as_bytes());
        }
        let (ctype, body) = mp.finish();
        let r = cf
            .agent
            .post(cf.url("/workers/assets/upload?base64=true"))
            .header(
                "Authorization",
                format!("Bearer {}", s.jwt.as_deref().unwrap_or_default()),
            )
            .header("Content-Type", ctype)
            .send(&body[..])?;
        let done: Done = cf.read(r)?.unwrap_or_default();
        if let Some(j) = done.jwt {
            jwt = j;
        }
    }
    if jwt.is_empty() {
        return Err(CfError::Api(
            "upload dos assets sem token de conclusão".into(),
        ));
    }
    Ok(jwt)
}

/// Publica o Worker (PUT do script com os módulos e os bindings).
fn upload_worker(
    cf: &Cf,
    b: &Bundle,
    worker: &str,
    bindings: Vec<Value>,
    keep: &[&str],
    assets_jwt: &str,
) -> Result<()> {
    let mut config = json!({ "compatibility_date": b.compatibility_date });
    if let Some(h) = &b.headers {
        config["_headers"] = json!(h);
    }
    let mut meta = json!({
        "main_module": b.main_module,
        "compatibility_date": b.compatibility_date,
        "bindings": bindings,
        "assets": { "jwt": assets_jwt, "config": config },
        "observability": { "enabled": true },
    });
    if !keep.is_empty() {
        meta["keep_bindings"] = json!(keep);
    }
    let mut mp = Multipart::new();
    mp.part(
        "metadata",
        None,
        "application/json",
        meta.to_string().as_bytes(),
    );
    for m in &b.modules {
        let data = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &m.b64)
            .map_err(|e| CfError::Api(format!("módulo {} inválido: {e}", m.name)))?;
        mp.part(&m.name, Some(&m.name), &m.kind, &data);
    }
    let (ctype, body) = mp.finish();
    let r = cf
        .agent
        .put(cf.url(&format!("/workers/scripts/{worker}")))
        .header("Authorization", format!("Bearer {}", cf.token))
        .header("Content-Type", ctype)
        .send(&body[..])?;
    Cf::must(cf.read::<Value>(r)?, "worker")?;
    Ok(())
}

/// Cron da faxina diária.
fn schedules(cf: &Cf, b: &Bundle, worker: &str) -> Result<()> {
    let crons: Vec<Value> = b.crons.iter().map(|c| json!({ "cron": c })).collect();
    cf.send::<Value>(
        "PUT",
        &format!("/workers/scripts/{worker}/schedules"),
        &json!(crons),
    )?;
    Ok(())
}

/// Subdomínio `workers.dev` da conta (cria um se a conta ainda não tem).
fn subdomain(cf: &Cf, hint: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Sub {
        subdomain: String,
    }
    if let Some(s) = cf.get::<Sub>("/workers/subdomain")? {
        return Ok(s.subdomain);
    }
    let base: String = hint
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if base.len() < 3 {
        "glint".to_string()
    } else {
        base.chars().take(40).collect()
    };
    let mut last = None;
    for i in 0..4 {
        let name = if i == 0 {
            base.clone()
        } else {
            let tail: String = random_b64(6)
                .to_lowercase()
                .chars()
                .filter(char::is_ascii_alphanumeric)
                .take(5)
                .collect();
            format!("{base}-{tail}")
        };
        match cf.send::<Sub>("PUT", "/workers/subdomain", &json!({ "subdomain": name })) {
            Ok(s) => return Ok(s.subdomain),
            Err(e @ (CfError::BadToken | CfError::Cloud(_))) => return Err(e),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or(CfError::NotFound))
}

/// Liga o `<worker>.<subdomínio>.workers.dev` do script.
fn enable_workers_dev(cf: &Cf, worker: &str) -> Result<()> {
    let r = cf
        .agent
        .post(cf.url(&format!("/workers/scripts/{worker}/subdomain")))
        .header("Authorization", format!("Bearer {}", cf.token))
        .header("Cloudflare-Workers-Script-Api-Date", "2025-08-01")
        .send_json(json!({ "enabled": true, "previews_enabled": false }))?;
    cf.read::<Value>(r)?;
    Ok(())
}

/// Espera o endereço novo responder (subdomínio novo pode levar alguns minutos).
fn wait_online(url: &str) -> Result<()> {
    let client = Client::new(url, None);
    let until = Instant::now() + Duration::from_secs(300);
    loop {
        match client.info() {
            Ok(i) if i.api_version == API_VERSION => return Ok(()),
            _ if Instant::now() > until => return Err(CfError::Timeout),
            _ => std::thread::sleep(Duration::from_secs(5)),
        }
    }
}

fn plain(name: &str, text: &str) -> Value {
    json!({ "type": "plain_text", "name": name, "text": text })
}

fn secret(name: &str, text: &str) -> Value {
    json!({ "type": "secret_text", "name": name, "text": text })
}

/// Cria o servidor na conta do token e espera ele responder → URL (`https://...workers.dev`).
/// Depois disso o app chama `POST /api/v1/setup` com `new.setup_token`.
pub fn create(
    token: &str,
    b: &Bundle,
    new: &NewServer,
    on: &mut dyn FnMut(Step),
) -> Result<String> {
    on(Step::Account);
    let account = accounts(token)?
        .into_iter()
        .next()
        .ok_or(CfError::NoAccount)?;
    let cf = connect(token, account);
    let worker = &b.names.worker;
    let exists = cf
        .get::<Value>(&format!("/workers/scripts/{worker}/settings"))?
        .is_some();
    if exists && !new.resume {
        return Err(CfError::Exists);
    }

    on(Step::Database);
    let db = database(&cf, &b.names.database)?;
    migrate(&cf, &db, &b.migrations)?;

    on(Step::Storage);
    bucket(&cf, &b.names.bucket)?;

    on(Step::Upload);
    let jwt = upload_assets(&cf, worker, b)?;
    let mut bindings = vec![
        json!({ "type": "d1", "name": "DB", "id": db }),
        json!({ "type": "r2_bucket", "name": "BUCKET", "bucket_name": b.names.bucket }),
    ];
    let mut vars = b.vars.clone();
    vars.insert("SERVER_NAME".into(), new.name.trim().to_string());
    vars.insert("SERVER_LANG".into(), new.lang.clone());
    bindings.extend(vars.iter().map(|(k, v)| plain(k, v)));
    let keep: &[&str] = if exists {
        // Retomada: os segredos do primeiro envio continuam (SIGNING_KEY assina links já dados).
        &["secret_text", "secret_key"]
    } else {
        bindings.push(secret("SIGNING_KEY", &random_secret()));
        bindings.push(secret("SETUP_TOKEN", &new.setup_token));
        &[]
    };
    upload_worker(&cf, b, worker, bindings, keep, &jwt)?;

    on(Step::Publish);
    schedules(&cf, b, worker)?;
    let sub = subdomain(
        &cf,
        if new.name.trim().is_empty() {
            "glint"
        } else {
            &new.name
        },
    )?;
    enable_workers_dev(&cf, worker)?;
    let url = format!("https://{worker}.{sub}.workers.dev");

    on(Step::Waiting);
    wait_online(&url)?;
    Ok(url)
}

/// Atualiza o servidor de `server_url` (`https://<worker>.<subdomínio>.workers.dev`) para o pacote
/// `b`: migrações, assets e Worker. Variáveis e segredos configurados ficam como estão.
pub fn update(token: &str, b: &Bundle, server_url: &str, on: &mut dyn FnMut(Step)) -> Result<()> {
    let host = server_url
        .split("://")
        .nth(1)
        .unwrap_or(server_url)
        .split(['/', ':'])
        .next()
        .unwrap_or_default();
    let parts: Vec<&str> = host.split('.').collect();
    let [worker, sub, "workers", "dev"] = parts[..] else {
        return Err(CfError::NotWorkersDev);
    };

    on(Step::Account);
    #[derive(Deserialize)]
    struct Sub {
        subdomain: String,
    }
    let mut found = None;
    for account in accounts(token)? {
        let cf = connect(token, account);
        if cf
            .get::<Sub>("/workers/subdomain")?
            .is_some_and(|s| s.subdomain == sub)
        {
            found = Some(cf);
            break;
        }
    }
    let cf = found.ok_or(CfError::NotFound)?;

    #[derive(Deserialize)]
    struct Settings {
        #[serde(default)]
        bindings: Vec<Value>,
    }
    let settings: Settings = cf
        .get(&format!("/workers/scripts/{worker}/settings"))?
        .ok_or(CfError::NotFound)?;
    let binding = |name: &str| {
        settings
            .bindings
            .iter()
            .find(|v| v["name"] == name)
            .cloned()
    };
    let db = binding("DB")
        .and_then(|v| v["id"].as_str().map(str::to_string))
        .ok_or_else(|| CfError::Api("o Worker não tem o banco DB".into()))?;
    let bucket =
        binding("BUCKET").ok_or_else(|| CfError::Api("o Worker não tem o bucket BUCKET".into()))?;

    on(Step::Database);
    migrate(&cf, &db, &b.migrations)?;

    on(Step::Upload);
    let jwt = upload_assets(&cf, worker, b)?;
    let mut bindings = vec![
        json!({ "type": "d1", "name": "DB", "id": db }),
        json!({ "type": "r2_bucket", "name": "BUCKET", "bucket_name": bucket["bucket_name"] }),
    ];
    // Variável nova desta versão que o servidor ainda não tem: entra com o padrão.
    bindings.extend(
        b.vars
            .iter()
            .filter(|(k, _)| binding(k).is_none())
            .map(|(k, v)| plain(k, v)),
    );
    let keep = ["plain_text", "json", "secret_text", "secret_key"];
    upload_worker(&cf, b, worker, bindings, &keep, &jwt)?;

    on(Step::Publish);
    schedules(&cf, b, worker)?;
    Ok(())
}
