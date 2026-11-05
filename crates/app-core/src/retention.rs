//! Limite de disco da pasta de clipes (lógica pura).
//!
//! Regra: o total de **todos** os clipes conta para o
//! limite; ao passar, apagam-se os mais antigos que não são favoritos nem
//! publicados. O clipe mais novo nunca é apagado (acabou de ser salvo).
//! Se só favoritos/publicados já passam do limite, nada é apagado.

/// O mínimo sobre um clipe para decidir a limpeza.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipUsage {
    pub id: i64,
    pub size_bytes: u64,
    /// Criação em ms desde a época Unix.
    pub created_ms: i64,
    /// Favorito ou publicado: nunca apagar.
    pub protected: bool,
}

/// Ids a apagar (do mais antigo para o mais novo) para ficar dentro de `limit_bytes`.
pub fn plan_cleanup(clips: &[ClipUsage], limit_bytes: u64) -> Vec<i64> {
    let mut total: u64 = clips.iter().map(|c| c.size_bytes).sum();
    if total <= limit_bytes {
        return Vec::new();
    }
    let newest = clips
        .iter()
        .max_by_key(|c| (c.created_ms, c.id))
        .map(|c| c.id);
    let mut candidates: Vec<&ClipUsage> = clips
        .iter()
        .filter(|c| !c.protected && Some(c.id) != newest)
        .collect();
    candidates.sort_by_key(|c| (c.created_ms, c.id));
    let mut out = Vec::new();
    for c in candidates {
        if total <= limit_bytes {
            break;
        }
        total -= c.size_bytes;
        out.push(c.id);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: i64, mb: u64, t: i64, protected: bool) -> ClipUsage {
        ClipUsage {
            id,
            size_bytes: mb * 1_000_000,
            created_ms: t,
            protected,
        }
    }

    #[test]
    fn under_limit_deletes_nothing() {
        let clips = [clip(1, 100, 1, false), clip(2, 100, 2, false)];
        assert!(plan_cleanup(&clips, 200_000_000).is_empty());
    }

    #[test]
    fn deletes_oldest_unprotected_first() {
        let clips = [
            clip(3, 100, 30, false),
            clip(1, 100, 10, false),
            clip(2, 100, 20, true),
            clip(4, 100, 40, false),
        ];
        // 400 MB, limite 250 → precisa tirar 2 clipes: 1 (mais antigo) e 3 (o 2 é favorito).
        assert_eq!(plan_cleanup(&clips, 250_000_000), vec![1, 3]);
    }

    #[test]
    fn never_deletes_newest_nor_protected() {
        let clips = [clip(1, 500, 1, true), clip(2, 500, 2, false)];
        assert!(plan_cleanup(&clips, 100).is_empty());
        let clips = [clip(1, 500, 1, false), clip(2, 500, 2, false)];
        assert_eq!(plan_cleanup(&clips, 100), vec![1]);
    }
}
