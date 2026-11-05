//! Formatação de tempos, tamanhos e datas no padrão PT-BR (vírgula decimal).

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};

use crate::{tr, tr_args};

/// `mm:ss.d` — tempo de player/recorte (ex.: `00:12.4`).
pub fn clock(secs: f32) -> String {
    let tenths = (secs.max(0.0) * 10.0).round() as u32;
    let (m, rest) = (tenths / 600, tenths % 600);
    format!("{:02}:{:02}.{}", m, rest / 10, rest % 10)
}

/// `m:ss,d` — tempo na cápsula do player (ex.: `0:21,4`).
pub fn player_time(secs: f32) -> String {
    let tenths = (secs.max(0.0) * 10.0).round() as u32;
    let (m, rest) = (tenths / 600, tenths % 600);
    format!("{}:{:02},{}", m, rest / 10, rest % 10)
}

/// `m:ss` — duração curta de card e régua (ex.: `0:42`).
pub fn short(secs: f32) -> String {
    let s = secs.max(0.0).round() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Segundos com uma casa e vírgula (ex.: `18,7 s`).
pub fn seconds(secs: f32) -> String {
    format!("{} s", decimal(secs, 1))
}

/// Gigabytes com uma casa (ex.: `18,9 GB`).
pub fn gigabytes(gb: f32) -> String {
    let places = if gb.fract().abs() < 0.05 { 0 } else { 1 };
    format!("{} GB", decimal(gb, places))
}

/// Tamanho de arquivo (unidades decimais, como o limite de disco): `63,4 MB`, `171 MB`, `1,6 GB`.
pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb < 1.0 {
        return format!("{} KB", (bytes as f64 / 1e3).round() as u64);
    }
    if mb < 100.0 {
        return format!("{} MB", decimal(mb as f32, 1));
    }
    if mb < 1000.0 {
        return format!("{} MB", mb.round() as u64);
    }
    gigabytes((mb / 1000.0) as f32)
}

/// Número com `places` casas decimais (vírgula em português, ponto nas outras línguas).
pub fn decimal(v: f32, places: usize) -> String {
    let s = format!("{v:.places$}");
    if rust_i18n::locale().starts_with("pt") {
        s.replace('.', ",")
    } else {
        s
    }
}

/// Data local de um instante em ms.
pub fn local(ms: i64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(Local::now)
}

fn month_short(m: u32) -> String {
    tr(&format!("date.month_short.{m}")).to_string()
}

fn weekday(d: NaiveDate) -> String {
    tr(&format!(
        "date.weekday.{}",
        d.weekday().num_days_from_monday()
    ))
    .to_string()
}

/// "3 de nov." (com o ano se não for o ano de `today`).
pub fn day_month(d: NaiveDate, today: NaiveDate) -> String {
    let base = tr_args(
        "date.day_month",
        &[
            ("day", &d.day().to_string()),
            ("month", &month_short(d.month())),
        ],
    )
    .to_string();
    if d.year() == today.year() {
        base
    } else {
        format!("{base} {}", d.year())
    }
}

/// Data relativa do card: "agora", "há 5 min", "há 3 h", "ontem", "segunda", "3 de nov.".
pub fn relative(ms: i64, now_ms: i64) -> String {
    let mins = (now_ms - ms).max(0) / 60_000;
    let (then, now) = (local(ms).date_naive(), local(now_ms).date_naive());
    if mins < 1 {
        return tr("date.now").to_string();
    }
    if mins < 60 {
        return tr_args("date.minutes_ago", &[("n", &mins.to_string())]).to_string();
    }
    if then == now {
        return tr_args("date.hours_ago", &[("n", &(mins / 60).to_string())]).to_string();
    }
    let days = (now - then).num_days();
    if days == 1 {
        return tr("date.yesterday").to_string();
    }
    if (2..7).contains(&days) {
        return weekday(then);
    }
    day_month(then, now)
}

/// Separador de dia da biblioteca: "Hoje", "Ontem", "segunda, 2 de nov.".
pub fn day_label(d: NaiveDate, today: NaiveDate) -> String {
    match (today - d).num_days() {
        0 => tr("date.today").to_string(),
        1 => tr("date.yesterday_title").to_string(),
        _ => format!("{}, {}", weekday(d), day_month(d, today)),
    }
}

/// Quando foi gravado, curto, para o topo da tela do clipe: ("hoje", "21:14"),
/// ("ontem", …), ("segunda", …), ("3 de nov.", …).
pub fn when(ms: i64, now_ms: i64) -> (String, String) {
    let t = local(ms);
    let (then, now) = (t.date_naive(), local(now_ms).date_naive());
    let day = match (now - then).num_days() {
        0 => tr("date.today_lower").to_string(),
        1 => tr("date.yesterday").to_string(),
        2..=6 => weekday(then),
        _ => day_month(then, now),
    };
    (day, t.format("%H:%M").to_string())
}

/// Data completa do painel do clipe: "4 de nov. de 2026, 21:30".
pub fn full_date(ms: i64) -> String {
    let t = local(ms);
    tr_args(
        "date.full",
        &[
            ("day", &t.day().to_string()),
            ("month", &month_short(t.month())),
            ("year", &t.year().to_string()),
            ("time", &t.format("%H:%M").to_string()),
        ],
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relogio() {
        assert_eq!(clock(0.0), "00:00.0");
        assert_eq!(clock(12.44), "00:12.4");
        assert_eq!(clock(59.96), "01:00.0");
        assert_eq!(clock(61.25), "01:01.3");
        assert_eq!(clock(-3.0), "00:00.0");
    }

    #[test]
    fn tempo_do_player() {
        assert_eq!(player_time(21.44), "0:21,4");
        assert_eq!(player_time(62.0), "1:02,0");
    }

    #[test]
    fn curto() {
        assert_eq!(short(42.0), "0:42");
        assert_eq!(short(60.0), "1:00");
        assert_eq!(short(9.4), "0:09");
    }

    #[test]
    fn decimais_com_virgula() {
        rust_i18n::set_locale("pt-BR");
        assert_eq!(seconds(18.7), "18,7 s");
        assert_eq!(gigabytes(18.94), "18,9 GB");
        assert_eq!(gigabytes(50.0), "50 GB");
        assert_eq!(decimal(3.0, 0), "3");
    }

    #[test]
    fn tamanhos() {
        rust_i18n::set_locale("pt-BR");
        assert_eq!(size(512_000), "512 KB");
        assert_eq!(size(63_400_000), "63,4 MB");
        assert_eq!(size(171_200_000), "171 MB");
        assert_eq!(size(1_600_000_000), "1,6 GB");
    }

    #[test]
    fn datas_relativas() {
        rust_i18n::set_locale("pt-BR");
        let now = Local.with_ymd_and_hms(2026, 11, 4, 21, 0, 0).unwrap();
        let ms = |t: DateTime<Local>| t.timestamp_millis();
        let n = ms(now);
        assert_eq!(relative(n - 20_000, n), "agora");
        assert_eq!(relative(n - 5 * 60_000, n), "há 5 min");
        assert_eq!(relative(n - 3 * 3_600_000, n), "há 3 h");
        let ontem = Local.with_ymd_and_hms(2026, 11, 3, 23, 0, 0).unwrap();
        assert_eq!(relative(ms(ontem), n), "ontem");
        let seg = Local.with_ymd_and_hms(2026, 11, 2, 10, 0, 0).unwrap();
        assert_eq!(relative(ms(seg), n), "segunda");
        let velho = Local.with_ymd_and_hms(2026, 10, 1, 10, 0, 0).unwrap();
        assert_eq!(relative(ms(velho), n), "1 de out.");
        let ano = Local.with_ymd_and_hms(2025, 10, 1, 10, 0, 0).unwrap();
        assert_eq!(relative(ms(ano), n), "1 de out. 2025");

        let hoje = now.date_naive();
        assert_eq!(day_label(hoje, hoje), "Hoje");
        assert_eq!(day_label(ontem.date_naive(), hoje), "Ontem");
        assert_eq!(day_label(seg.date_naive(), hoje), "segunda, 2 de nov.");
        assert_eq!(full_date(ms(now)), "4 de nov. de 2026, 21:00");
        assert_eq!(when(n - 60_000, n), ("hoje".into(), "20:59".into()));
        assert_eq!(when(ms(ontem), n).0, "ontem");
        assert_eq!(when(ms(velho), n).0, "1 de out.");
    }
}
