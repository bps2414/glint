use std::sync::Arc;

/// Índice da stream dentro do clipe (0 = vídeo, 1.. = trilhas de áudio).
pub type StreamId = usize;

/// Um pacote já comprimido (H.264/HEVC/AAC...) pronto para ser muxado.
///
/// O buffer de replay guarda **somente** pacotes comprimidos: 60 s de
/// 1080p60 HEVC ocupam ~150–225 MB, contra dezenas de GB em frames crus.
#[derive(Clone, Debug)]
pub struct EncodedPacket {
    pub stream: StreamId,
    /// PTS na time base da stream.
    pub pts: i64,
    /// DTS na time base da stream.
    pub dts: i64,
    /// Duração na time base da stream (0 se desconhecida).
    pub duration: i64,
    /// Instante de apresentação em microssegundos no relógio comum
    /// (mesma origem para vídeo e áudio). Usado para cortar o buffer.
    pub time_us: i64,
    pub keyframe: bool,
    pub data: Arc<[u8]>,
}

impl EncodedPacket {
    pub fn size(&self) -> usize {
        self.data.len()
    }
}
