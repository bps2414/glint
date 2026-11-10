//! Chamadas da API v1 e tipos do contrato.

use std::io::Read;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ureq::http::Response;
use ureq::{Agent, Body, SendBody};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Private,
    Unlisted,
    Public,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Clip {
    pub id: String,
    pub title: String,
    pub game: Option<String>,
    pub visibility: Visibility,
    pub status: String,
    pub size_bytes: u64,
    pub duration_ms: i64,
    pub url: Option<String>,
    /// Servidor antigo não manda.
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub created_at: i64,
}

/// `GET /clips/cleanup`: os clipes mais antigos (sem favoritos) que liberariam o espaço pedido.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Cleanup {
    pub clips: Vec<Clip>,
    pub freed_bytes: u64,
    /// `false` = nem apagando todos estes cabe.
    pub enough: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct UploadState {
    pub part_size: u64,
    pub part_count: u32,
    pub parts_done: Vec<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NewClip {
    pub title: String,
    pub game: Option<String>,
    pub visibility: Visibility,
    pub size_bytes: u64,
    pub duration_ms: i64,
    pub width: u32,
    pub height: u32,
    /// Favorito no app: nunca entra na sugestão do que apagar quando o espaço acaba.
    pub favorite: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct User {
    /// Nome no servidor (conta por convite). Servidor antigo não manda.
    #[serde(default)]
    pub name: Option<String>,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// `"admin"` ou `"user"`.
    #[serde(default)]
    pub role: Option<String>,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role.as_deref() == Some("admin")
    }
}

/// `GET /api/v1/info`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub server_version: String,
    pub api_version: u32,
    #[serde(default)]
    pub discord_login: bool,
    #[serde(default)]
    pub setup_required: bool,
}

/// Versão da API que este app fala.
pub const API_VERSION: u32 = 1;

/// Aparelho (sessão) de um membro.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Device {
    pub id: String,
    /// `"app"` ou `"web"`.
    pub kind: String,
    pub last_used_at: i64,
    pub current: bool,
}

/// Membro do servidor (`GET /api/v1/members`, só admin).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Member {
    pub id: String,
    #[serde(flatten)]
    pub user: User,
    pub disabled: bool,
    pub used_bytes: u64,
    pub clips: u32,
    pub devices: Vec<Device>,
}

/// Código curto para entrar com a mesma conta em outro PC.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct LinkCode {
    pub code: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Quota {
    pub used_bytes: u64,
    pub quota_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Me {
    pub user: User,
    pub quota: Quota,
}

impl Me {
    pub fn name(&self) -> &str {
        self.user
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .or(self.user.display_name.as_deref().filter(|n| !n.is_empty()))
            .unwrap_or(&self.user.username)
    }
}

/// Corpo de erro do servidor (`message` já vem em PT-BR, pronta para mostrar).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
    /// Só em `storage_full` (507) e `quota_exceeded` (409).
    #[serde(default)]
    pub used_bytes: Option<u64>,
    #[serde(default)]
    pub limit_bytes: Option<u64>,
    #[serde(default)]
    pub quota_bytes: Option<u64>,
    /// Quanto falta liberar para o clipe caber (servidor 1.1+).
    #[serde(default)]
    pub needed_bytes: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    /// Token vencido ou revogado: refazer o login.
    #[error("Sua sessão expirou. Entre de novo.")]
    Unauthorized,
    #[error("{}", .body.message)]
    Api { status: u16, body: ApiErrorBody },
    /// 507 `storage_full` (o servidor inteiro passaria do limite) ou 409 `quota_exceeded`
    /// (`quota`: a cota do usuário). Tentar de novo só adianta liberando `needed_bytes`.
    #[error("O servidor está cheio.")]
    StorageFull {
        used_bytes: u64,
        limit_bytes: u64,
        /// `None` = servidor antigo (não sugere o que apagar).
        needed_bytes: Option<u64>,
        quota: bool,
    },
    #[error("Sem conexão com o servidor ({0}).")]
    Network(String),
    #[error("cancelado")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl CloudError {
    /// Vale tentar de novo (rede, 5xx, limite de requisições).
    pub fn retryable(&self) -> bool {
        match self {
            CloudError::Network(_) => true,
            CloudError::Api { status, .. } => *status >= 500 || *status == 429,
            _ => false,
        }
    }
}

impl From<ureq::Error> for CloudError {
    fn from(e: ureq::Error) -> Self {
        CloudError::Network(e.to_string())
    }
}

impl From<std::io::Error> for CloudError {
    fn from(e: std::io::Error) -> Self {
        CloudError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CloudError>;

/// Id do clipe a partir do link publicado (`https://.../c/<id>`).
pub fn clip_id_from_url(url: &str) -> Option<&str> {
    let rest = url.split("/c/").nth(1)?;
    let id = rest.split(['/', '?', '#']).next()?;
    (!id.is_empty()).then_some(id)
}

#[derive(Clone)]
pub struct Client {
    base: String,
    token: Option<String>,
    agent: Agent,
}

#[derive(Deserialize)]
struct ErrWrap {
    error: ApiErrorBody,
}
#[derive(Deserialize)]
struct ClipWrap {
    clip: Clip,
}
#[derive(Deserialize)]
struct CreateResp {
    clip: Clip,
    upload: UploadState,
}
#[derive(Deserialize)]
struct GetResp {
    clip: Clip,
    upload: Option<UploadState>,
}
#[derive(Deserialize)]
struct TokenResp {
    token: String,
}
#[derive(Deserialize)]
struct InviteWrap {
    invite: InviteUrl,
}
#[derive(Deserialize)]
struct InviteUrl {
    url: String,
}
#[derive(Deserialize)]
struct MembersResp {
    members: Vec<Member>,
}
#[derive(Deserialize)]
struct UrlResp {
    url: String,
}

impl Client {
    pub fn new(base: &str, token: Option<String>) -> Self {
        let agent: Agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .build()
            .into();
        Self {
            base: base.trim_end_matches('/').to_string(),
            token,
            agent,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn auth(&self) -> Result<String> {
        self.token
            .as_ref()
            .map(|t| format!("Bearer {t}"))
            .ok_or(CloudError::Unauthorized)
    }

    /// Erro do servidor → [`CloudError`]; sucesso passa adiante.
    fn check(mut resp: Response<Body>) -> Result<Response<Body>> {
        let status = resp.status().as_u16();
        if status < 400 {
            return Ok(resp);
        }
        if status == 401 {
            return Err(CloudError::Unauthorized);
        }
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let body = serde_json::from_str::<ErrWrap>(&text)
            .map(|w| w.error)
            .unwrap_or_else(|_| ApiErrorBody {
                code: "http".into(),
                message: format!("O servidor respondeu {status}."),
                used_bytes: None,
                limit_bytes: None,
                quota_bytes: None,
                needed_bytes: None,
            });
        if status == 507 && body.code == "storage_full" {
            return Err(CloudError::StorageFull {
                used_bytes: body.used_bytes.unwrap_or(0),
                limit_bytes: body.limit_bytes.unwrap_or(0),
                needed_bytes: body.needed_bytes,
                quota: false,
            });
        }
        if status == 409 && body.code == "quota_exceeded" && body.needed_bytes.is_some() {
            return Err(CloudError::StorageFull {
                used_bytes: body.used_bytes.unwrap_or(0),
                limit_bytes: body.quota_bytes.unwrap_or(0),
                needed_bytes: body.needed_bytes,
                quota: true,
            });
        }
        Err(CloudError::Api { status, body })
    }

    pub(crate) fn json<T: for<'de> Deserialize<'de>>(resp: Response<Body>) -> Result<T> {
        let mut resp = Self::check(resp)?;
        resp.body_mut()
            .read_json::<T>()
            .map_err(|e| CloudError::Other(format!("resposta inválida do servidor: {e}")))
    }

    /// `POST /api/v1/auth/token` (passo 5 do login).
    pub fn exchange_code(&self, code: &str, verifier: &str) -> Result<String> {
        let r = self
            .agent
            .post(self.url("/api/v1/auth/token"))
            .send_json(serde_json::json!({ "code": code, "verifier": verifier }))?;
        Ok(Self::json::<TokenResp>(r)?.token)
    }

    /// `GET /api/v1/info` (público).
    pub fn info(&self) -> Result<ServerInfo> {
        let r = self.agent.get(self.url("/api/v1/info")).call()?;
        Self::json(r)
    }

    /// `POST /api/v1/join`: gasta o convite (código ou link) e cria a conta → token.
    pub fn join(&self, invite: &str, name: &str) -> Result<String> {
        let r = self
            .agent
            .post(self.url("/api/v1/join"))
            .send_json(serde_json::json!({ "invite": invite, "name": name }))?;
        Ok(Self::json::<TokenResp>(r)?.token)
    }

    /// `POST /api/v1/setup`: cria o admin do servidor novo com o `SETUP_TOKEN` → token.
    pub fn setup(&self, setup_token: &str, name: &str) -> Result<String> {
        let r = self
            .agent
            .post(self.url("/api/v1/setup"))
            .send_json(serde_json::json!({ "setup_token": setup_token, "name": name }))?;
        Ok(Self::json::<TokenResp>(r)?.token)
    }

    /// `POST /api/v1/devices/link/claim`: código do outro PC → token da mesma conta.
    pub fn claim_link(&self, code: &str) -> Result<String> {
        let r = self
            .agent
            .post(self.url("/api/v1/devices/link/claim"))
            .send_json(serde_json::json!({ "code": code }))?;
        Ok(Self::json::<TokenResp>(r)?.token)
    }

    /// `POST /api/v1/devices/link`: código para entrar em outro PC (10 min, uso único).
    pub fn link_code(&self) -> Result<LinkCode> {
        let r = self
            .agent
            .post(self.url("/api/v1/devices/link"))
            .header("Authorization", &self.auth()?)
            .send_empty()?;
        Self::json(r)
    }

    /// `POST /api/v1/auth/web-code`: link de uso único que abre o site já logado em `next`.
    pub fn web_url(&self, next: &str) -> Result<String> {
        let r = self
            .agent
            .post(self.url("/api/v1/auth/web-code"))
            .header("Authorization", &self.auth()?)
            .send_json(serde_json::json!({ "next": next }))?;
        Ok(Self::json::<UrlResp>(r)?.url)
    }

    /// `POST /api/v1/invites` (admin): convite padrão (uso único, 7 dias) → link.
    pub fn create_invite(&self) -> Result<String> {
        let r = self
            .agent
            .post(self.url("/api/v1/invites"))
            .header("Authorization", &self.auth()?)
            .send_json(serde_json::json!({}))?;
        Ok(Self::json::<InviteWrap>(r)?.invite.url)
    }

    /// `GET /api/v1/members` (admin).
    pub fn members(&self) -> Result<Vec<Member>> {
        let r = self
            .agent
            .get(self.url("/api/v1/members"))
            .header("Authorization", &self.auth()?)
            .call()?;
        Ok(Self::json::<MembersResp>(r)?.members)
    }

    /// Desativa (`false`) ou reativa (`true`) um membro (admin).
    pub fn set_member_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let action = if enabled { "enable" } else { "disable" };
        let r = self
            .agent
            .post(self.url(&format!("/api/v1/members/{id}/{action}")))
            .header("Authorization", &self.auth()?)
            .send_empty()?;
        Self::check(r).map(|_| ())
    }

    /// Revoga um aparelho (sessão) de um membro (admin).
    pub fn revoke_device(&self, member: &str, device: &str) -> Result<()> {
        let r = self
            .agent
            .delete(self.url(&format!("/api/v1/members/{member}/devices/{device}")))
            .header("Authorization", &self.auth()?)
            .call()?;
        Self::check(r).map(|_| ())
    }

    pub fn logout(&self) -> Result<()> {
        let r = self
            .agent
            .post(self.url("/api/v1/auth/logout"))
            .header("Authorization", &self.auth()?)
            .send_empty()?;
        Self::check(r).map(|_| ())
    }

    pub fn me(&self) -> Result<Me> {
        let r = self
            .agent
            .get(self.url("/api/v1/me"))
            .header("Authorization", &self.auth()?)
            .call()?;
        Self::json(r)
    }

    pub fn create_clip(&self, meta: &NewClip) -> Result<(Clip, UploadState)> {
        let r = self
            .agent
            .post(self.url("/api/v1/clips"))
            .header("Authorization", &self.auth()?)
            .send_json(meta)?;
        let c: CreateResp = Self::json(r)?;
        Ok((c.clip, c.upload))
    }

    pub fn get_clip(&self, id: &str) -> Result<(Clip, Option<UploadState>)> {
        let r = self
            .agent
            .get(self.url(&format!("/api/v1/clips/{id}")))
            .header("Authorization", &self.auth()?)
            .call()?;
        let g: GetResp = Self::json(r)?;
        Ok((g.clip, g.upload))
    }

    /// `PUT .../parts/<n>`: `len` bytes lidos de `body`.
    pub fn put_part(&self, id: &str, n: u32, len: u64, body: &mut dyn Read) -> Result<()> {
        let r = self
            .agent
            .put(self.url(&format!("/api/v1/clips/{id}/parts/{n}")))
            .header("Authorization", &self.auth()?)
            .header("Content-Type", "application/octet-stream")
            .header("Content-Length", &len.to_string())
            .send(SendBody::from_reader(body))?;
        Self::check(r).map(|_| ())
    }

    pub fn put_thumb(&self, id: &str, jpeg: &[u8]) -> Result<()> {
        let r = self
            .agent
            .put(self.url(&format!("/api/v1/clips/{id}/thumb")))
            .header("Authorization", &self.auth()?)
            .header("Content-Type", "image/jpeg")
            .send(jpeg)?;
        Self::check(r).map(|_| ())
    }

    pub fn complete(&self, id: &str) -> Result<Clip> {
        let r = self
            .agent
            .post(self.url(&format!("/api/v1/clips/{id}/complete")))
            .header("Authorization", &self.auth()?)
            .send_empty()?;
        Ok(Self::json::<ClipWrap>(r)?.clip)
    }

    pub fn update_clip(
        &self,
        id: &str,
        title: Option<&str>,
        visibility: Option<Visibility>,
    ) -> Result<Clip> {
        let mut body = serde_json::Map::new();
        if let Some(t) = title {
            body.insert("title".into(), t.into());
        }
        if let Some(v) = visibility {
            body.insert("visibility".into(), serde_json::to_value(v).expect("enum"));
        }
        let r = self
            .agent
            .patch(self.url(&format!("/api/v1/clips/{id}")))
            .header("Authorization", &self.auth()?)
            .send_json(serde_json::Value::Object(body))?;
        Ok(Self::json::<ClipWrap>(r)?.clip)
    }

    /// Marca ou desmarca o favorito no servidor (favorito nunca é sugerido para apagar).
    pub fn set_favorite(&self, id: &str, favorite: bool) -> Result<Clip> {
        let r = self
            .agent
            .patch(self.url(&format!("/api/v1/clips/{id}")))
            .header("Authorization", &self.auth()?)
            .send_json(serde_json::json!({ "favorite": favorite }))?;
        Ok(Self::json::<ClipWrap>(r)?.clip)
    }

    /// `GET /clips/cleanup`: o que apagar para liberar `bytes` (o servidor não apaga nada).
    pub fn cleanup(&self, bytes: u64) -> Result<Cleanup> {
        let r = self
            .agent
            .get(self.url(&format!("/api/v1/clips/cleanup?bytes={bytes}")))
            .header("Authorization", &self.auth()?)
            .call()?;
        Self::json(r)
    }

    pub fn delete_clip(&self, id: &str) -> Result<()> {
        let r = self
            .agent
            .delete(self.url(&format!("/api/v1/clips/{id}")))
            .header("Authorization", &self.auth()?)
            .call()?;
        Self::check(r).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::{clip_id_from_url, Visibility};

    #[test]
    fn id_do_link() {
        assert_eq!(
            clip_id_from_url("https://x.workers.dev/c/AbC_123-xyz"),
            Some("AbC_123-xyz")
        );
        assert_eq!(clip_id_from_url("https://x.dev/c/abc?t=1"), Some("abc"));
        assert_eq!(clip_id_from_url("https://x.dev/"), None);
        assert_eq!(clip_id_from_url("https://x.dev/c/"), None);
    }

    #[test]
    fn visibilidade_em_minusculas() {
        assert_eq!(
            serde_json::to_string(&Visibility::Unlisted).unwrap(),
            "\"unlisted\""
        );
    }
}
