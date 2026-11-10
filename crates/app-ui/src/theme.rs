//! Design system do app (Fase 5, Liquid Glass neutro): paleta, escala de tamanhos,
//! fontes e a troca de tema ao vivo.
//!
//! Fonte da verdade visual: estes tokens (Liquid Glass neutro).
//! As cores dependem do tema do Windows (claro/escuro) e de "Efeitos de transparência":
//! por isso os tokens são funções (`tok::fg()`) que leem a paleta ativa, e não constantes.
//! Tamanhos e raios só vêm de [`size`] e dos `R*`: nada de altura solta nas telas, para os
//! controles ficarem alinhados e do mesmo tamanho em todo lugar.

use std::borrow::Cow;
use std::cell::Cell;

use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::*;

pub use glass::Palette;

thread_local! {
    static PAL: Cell<Palette> = const { Cell::new(glass::DARK) };
    static LOOK: Cell<Option<Look>> = const { Cell::new(None) };
}

/// Aparência ativa (antes da janela abrir: escuro com vidro).
pub fn look() -> Look {
    LOOK.with(Cell::get).unwrap_or(Look {
        dark: true,
        glass: true,
    })
}

/// Paleta ativa (a UI só roda na thread principal).
pub fn pal() -> Palette {
    PAL.with(Cell::get)
}

/// Cores da paleta ativa. Use direto: `.text_color(tok::fg())`.
#[allow(dead_code)] // parte entra em uso com os componentes de vidro (Parte 4)
pub mod tok {
    use gpui_kit::{rgba, Rgba};

    use super::pal;

    /// Fundo da janela (translúcido sobre o acrílico, ou sólido).
    pub fn bg() -> Rgba {
        rgba(pal().window)
    }
    /// Painel de vidro (sidebar, linha do tempo, publicar).
    pub fn panel() -> Rgba {
        rgba(pal().panel)
    }
    /// Superfície de controles (pílulas, campos, botões secundários).
    pub fn surf() -> Rgba {
        rgba(pal().pill)
    }
    /// Item selecionado / superfície elevada.
    pub fn surf_2() -> Rgba {
        rgba(pal().sel)
    }
    /// Caixa rebaixada (status do gravador, trilho de barra).
    pub fn inset() -> Rgba {
        rgba(pal().inset)
    }
    /// Campo de texto (título, link) e trilho do segmentado.
    pub fn field() -> Rgba {
        rgba(pal().field)
    }
    /// Trilho de barra de progresso.
    pub fn track() -> Rgba {
        rgba(pal().track)
    }
    /// Borda de painel/pílula (clara no vidro).
    pub fn edge() -> Rgba {
        rgba(pal().panel_border)
    }
    /// Linha divisória fina.
    pub fn line() -> Rgba {
        rgba(pal().line)
    }
    /// Linha forte (contorno de tecla, trilho).
    pub fn line_2() -> Rgba {
        rgba(pal().line_2)
    }
    pub fn fg() -> Rgba {
        rgba(pal().text)
    }
    pub fn fg_2() -> Rgba {
        rgba(pal().text2)
    }
    /// Texto apagado (metadados).
    pub fn muted() -> Rgba {
        rgba(pal().text3)
    }
    /// Azul: foco, progresso, seleção. Nada mais.
    pub fn blue() -> Rgba {
        rgba(pal().blue)
    }
    /// Vermelho: só sinal (gravando, erro, confirmar apagar).
    pub fn red() -> Rgba {
        rgba(pal().red)
    }
    pub fn red_halo() -> Rgba {
        rgba(pal().red_halo)
    }
    /// Botão principal (neutro: branco no escuro, grafite no claro).
    pub fn primary_top() -> Rgba {
        rgba(pal().primary_top)
    }
    pub fn primary_bot() -> Rgba {
        rgba(pal().primary_bot)
    }
    pub fn primary_fg() -> Rgba {
        rgba(pal().primary_fg)
    }
    /// Chip de vidro sobre imagem/vídeo (igual nos dois temas).
    pub fn chip() -> Rgba {
        rgba(0x1414184d)
    }
    pub fn chip_edge() -> Rgba {
        rgba(0xffffff38)
    }
    /// Texto sobre imagem/vídeo.
    pub fn on_media() -> Rgba {
        rgba(0xffffffff)
    }
    /// Fundo do player.
    pub fn black() -> Rgba {
        rgba(0x000000ff)
    }
    /// Área fora do recorte na linha do tempo.
    pub fn outside() -> Rgba {
        rgba(pal().dim)
    }
    /// Alça e moldura do recorte (branca nos dois temas).
    pub fn handle() -> Rgba {
        rgba(pal().handle)
    }
    pub fn handle_grip() -> Rgba {
        rgba(pal().handle_grip)
    }
    /// Cápsula do player sobre o vídeo (igual nos dois temas): base, realce e borda.
    pub fn capsule() -> Rgba {
        rgba(0x12121647)
    }
    pub fn capsule_hi() -> Rgba {
        rgba(0xffffff33)
    }
    pub fn capsule_edge() -> Rgba {
        rgba(0xffffff42)
    }
    /// Item ligado dentro da cápsula e divisória.
    pub fn capsule_on() -> Rgba {
        rgba(0xffffff38)
    }
}

/// Escala do design system. Toda altura de controle, ícone e espaçamento sai daqui, para
/// os controles ficarem alinhados e do mesmo tamanho em todo lugar (o mockup misturava
/// 34/36/38/40; aqui são quatro alturas só).
pub mod size {
    use gpui_kit::{px, Pixels};

    /// Tamanho de controle (botão, pílula, campo, botão redondo).
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub enum Size {
        /// 24: chips sobre imagem.
        Xs,
        /// 32: botões da janela, itens do segmentado, botões dentro de pílula, teclas.
        Sm,
        /// 40: padrão (pílulas da barra, campos, botões redondos, botões comuns).
        #[default]
        Md,
        /// 46: ação principal da tela.
        Lg,
    }

    impl Size {
        pub fn h(self) -> Pixels {
            px(match self {
                Size::Xs => 24.,
                Size::Sm => 32.,
                Size::Md => 40.,
                Size::Lg => 46.,
            })
        }

        /// Ícone dentro do controle.
        pub fn icon(self) -> Pixels {
            px(match self {
                Size::Xs => 12.,
                Size::Sm => 14.,
                Size::Md | Size::Lg => 16.,
            })
        }

        /// Folga lateral do texto.
        pub fn pad(self) -> Pixels {
            px(match self {
                Size::Xs => 8.,
                Size::Sm => 12.,
                Size::Md => 14.,
                Size::Lg => 20.,
            })
        }

        pub fn text(self) -> Pixels {
            px(match self {
                Size::Xs => 11.,
                Size::Sm => 12.,
                Size::Md => 13.,
                Size::Lg => 15.,
            })
        }
    }

    /// Item da sidebar (navegação e jogos).
    pub const NAV_ITEM: Pixels = px(36.);
    /// Ícone da sidebar.
    pub const NAV_ICON: Pixels = px(16.);
    /// Margem entre painéis e a borda da janela.
    #[allow(dead_code)] // layout flutuante (Parte 5)
    pub const INSET: Pixels = px(10.);
}

/// Raio de cards (miniaturas).
pub const R: Pixels = px(16.);
/// Raio de campos e itens de lista.
pub const R_SM: Pixels = px(12.);
/// Raio de painéis flutuantes (sidebar).
#[allow(dead_code)] // sidebar flutuante (Parte 5)
pub const R_PANEL: Pixels = px(22.);
/// Raio de painéis menores e do player.
#[allow(dead_code)]
pub const R_PANEL_SM: Pixels = px(20.);

/// Família da interface.
pub const FONT_UI: &str = "Inter";
/// Família dos números (tempos, tamanhos, contagens).
pub const FONT_NUM: &str = "Geist Mono";

/// Durações de animação (≤ 250 ms nas transições; leveza vence efeito).
pub mod motion {
    use std::time::Duration;
    /// Pulso do ponto de gravação.
    pub const REC_PULSE: Duration = Duration::from_millis(1800);
    /// Transições curtas (hover, toggle).
    pub const FAST: Duration = Duration::from_millis(150);
}

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Regular.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Medium.ttf"),
];

const DARK_NAME: &str = "Glint Escuro";
const LIGHT_NAME: &str = "Glint Claro";
const DARK_JSON: &str = include_str!("../assets/themes/glint-dark.json");
const LIGHT_JSON: &str = include_str!("../assets/themes/glint-light.json");

/// Registra as fontes e os dois temas do gpui-component. Chamar depois de `gpui_kit::init`.
pub fn install(cx: &mut App) -> anyhow::Result<()> {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())?;

    let registry = ThemeRegistry::global_mut(cx);
    registry.load_themes_from_str(DARK_JSON)?;
    registry.load_themes_from_str(LIGHT_JSON)?;
    let get = |name: &str| {
        registry
            .themes()
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("tema {name} não encontrado"))
    };
    let (dark, light) = (get(DARK_NAME)?, get(LIGHT_NAME)?);
    let theme = Theme::global_mut(cx);
    theme.dark_theme = dark;
    theme.light_theme = light;
    Theme::change(ThemeMode::Dark, None, cx);
    // Barra de rolagem sempre à vista (dá pra clicar e arrastar), não só ao rolar.
    Theme::set_scrollbar_mode(ScrollbarMode::Always, cx);
    Ok(())
}

/// Como a janela deve aparecer agora.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Look {
    pub dark: bool,
    /// Vidro (acrílico) ligado; desligado = tudo sólido.
    pub glass: bool,
}

impl Look {
    /// Lê o tema da janela e as preferências do sistema.
    pub fn detect(window: &Window) -> Self {
        // `GLINT_THEME=dark|light` força o tema (prints e testes fora do Windows).
        let forced = std::env::var("GLINT_THEME").ok();
        let dark = match forced.as_deref() {
            Some("dark") => true,
            Some("light") => false,
            _ => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        Self {
            dark,
            glass: glass_allowed(),
        }
    }

    pub fn palette(self) -> Palette {
        let p = if self.dark { glass::DARK } else { glass::LIGHT };
        if self.glass {
            p
        } else {
            glass::solid(p)
        }
    }
}

/// "Efeitos de transparência" ligado e sem economia de bateria.
#[cfg(windows)]
fn glass_allowed() -> bool {
    use app_platform_win::appearance;
    appearance::transparency_enabled() && !appearance::battery_saver_on()
}

#[cfg(not(windows))]
fn glass_allowed() -> bool {
    true
}

/// Relê o tema do Windows e a transparência e aplica se mudou. Chamar ao abrir a janela,
/// quando o tema do sistema muda, quando a janela ganha foco e 1×/min (a transparência e a
/// economia de bateria não avisam quando mudam).
pub fn refresh(window: &mut Window, cx: &mut App) {
    let look = Look::detect(window);
    if LOOK.with(Cell::get) != Some(look) {
        apply(look, window, cx);
    }
}

/// Aplica a aparência: paleta, tema do gpui-component e fundo da janela.
pub fn apply(look: Look, window: &mut Window, cx: &mut App) {
    LOOK.with(|l| l.set(Some(look)));
    PAL.with(|p| p.set(look.palette()));
    let mode = if look.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, Some(window), cx);
    let bg: Hsla = if look.glass {
        // A raiz pinta `tok::bg()` por cima do acrílico.
        transparent_black()
    } else {
        tok::bg().into()
    };
    Theme::global_mut(cx).tokens.background = bg.into();
    window.set_background_appearance(if look.glass {
        WindowBackgroundAppearance::Blurred
    } else {
        WindowBackgroundAppearance::Opaque
    });
    window.refresh();
}

/// Paleta Liquid Glass. Cores em `0xRRGGBBAA`. Os gradientes de painel/pílula são
/// "realce sobre base": a base é a cor de baixo do gradiente do mockup e o realce é um
/// branco que some de cima para baixo.
pub mod glass {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Palette {
        pub dark: bool,
        /// Fundo da janela sobre o acrílico.
        pub window: u32,
        pub window_border: u32,
        /// Base do painel e o realce no topo.
        pub panel: u32,
        pub panel_hi: u32,
        pub panel_border: u32,
        /// Contorno extra fino (só no claro).
        pub panel_outline: u32,
        pub panel_shadow: u32,
        /// Pílula/botão: base e realce.
        pub pill: u32,
        pub pill_hi: u32,
        pub pill_shadow: u32,
        /// Item selecionado: base e realce.
        pub sel: u32,
        pub sel_hi: u32,
        pub sel_shadow: u32,
        /// Contorno do item selecionado.
        pub sel_outline: u32,
        /// Caixa rebaixada (status do gravador).
        pub inset: u32,
        /// Campo de texto e trilho do segmentado (painel Publicar).
        pub field: u32,
        pub track: u32,
        /// Fora do recorte na linha do tempo, alça do recorte e o risco da alça.
        pub dim: u32,
        pub handle: u32,
        pub handle_grip: u32,
        pub line: u32,
        pub line_2: u32,
        pub text: u32,
        pub text2: u32,
        pub text3: u32,
        /// Botão principal: topo, base e texto.
        pub primary_top: u32,
        pub primary_bot: u32,
        pub primary_fg: u32,
        pub blue: u32,
        pub red: u32,
        pub red_halo: u32,
        pub thumb_shadow: u32,
    }

    pub const DARK: Palette = Palette {
        dark: true,
        window: 0x14151994,
        window_border: 0xffffff1a,
        panel: 0xffffff09,
        panel_hi: 0xffffff11,
        panel_border: 0xffffff1c,
        panel_outline: 0x00000000,
        panel_shadow: 0x00000059,
        pill: 0xffffff0d,
        pill_hi: 0xffffff12,
        pill_shadow: 0x00000040,
        sel: 0xffffff14,
        sel_hi: 0xffffff15,
        sel_shadow: 0x00000040,
        sel_outline: 0xffffff2e,
        inset: 0x00000033,
        field: 0x00000038,
        track: 0xffffff1a,
        dim: 0x08090c9e,
        handle: 0xf5f5f7ff,
        handle_grip: 0x00000059,
        line: 0xffffff17,
        line_2: 0xffffff2e,
        text: 0xf5f5f7ff,
        text2: 0xebebf5a3,
        text3: 0xebebf573,
        primary_top: 0xffffffff,
        primary_bot: 0xe4e5e9ff,
        primary_fg: 0x111114ff,
        blue: 0x7fb0ffff,
        red: 0xff453aff,
        red_halo: 0xff453a2e,
        thumb_shadow: 0x00000059,
    };

    pub const LIGHT: Palette = Palette {
        dark: false,
        window: 0xf6f7fa8f,
        window_border: 0xffffffbf,
        panel: 0xffffff7a,
        panel_hi: 0xffffffb5,
        panel_border: 0xffffffe6,
        panel_outline: 0x0000000f,
        panel_shadow: 0x2832461f,
        pill: 0xffffff9e,
        pill_hi: 0xffffffa8,
        pill_shadow: 0x2832461a,
        sel: 0xffffffbf,
        sel_hi: 0xffffffff,
        sel_shadow: 0x2832461f,
        sel_outline: 0x00000014,
        inset: 0xffffff73,
        field: 0xffffff99,
        track: 0x00000014,
        dim: 0xf0f2f69e,
        handle: 0xffffffff,
        handle_grip: 0x0000004d,
        line: 0x00000014,
        line_2: 0x00000026,
        text: 0x111114ff,
        text2: 0x32323cb8,
        text3: 0x32323c80,
        primary_top: 0x2a2b30ff,
        primary_bot: 0x111114ff,
        primary_fg: 0xffffffff,
        blue: 0x2f6fe0ff,
        red: 0xe5322bff,
        red_halo: 0xe5322b29,
        thumb_shadow: 0x2832462e,
    };

    /// Mesma paleta sem transparência ("Efeitos de transparência" desligado / economia
    /// de bateria): as superfícies translúcidas viram opacas sobre o fundo sólido.
    pub fn solid(p: Palette) -> Palette {
        let base = if p.dark { 0x141518 } else { 0xf2f3f6 };
        let window = over(p.window, base);
        let on = |c: u32| over(c, window >> 8);
        Palette {
            window,
            panel: on(p.panel),
            pill: on(p.pill),
            sel: on(p.sel),
            inset: on(p.inset),
            field: on(p.field),
            ..p
        }
    }

    /// Compõe `c` (RRGGBBAA) sobre `base` (RRGGBB) e devolve opaco (RRGGBBff).
    pub fn over(c: u32, base: u32) -> u32 {
        let a = (c & 0xff) as f32 / 255.0;
        let ch = |shift_c: u32, shift_b: u32| {
            let s = ((c >> shift_c) & 0xff) as f32;
            let b = ((base >> shift_b) & 0xff) as f32;
            ((s * a + b * (1.0 - a)).round() as u32).min(255)
        };
        (ch(24, 16) << 24) | (ch(16, 8) << 16) | (ch(8, 0) << 8) | 0xff
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn solido_fica_opaco() {
            for p in [super::DARK, super::LIGHT] {
                let s = super::solid(p);
                for c in [s.window, s.panel, s.pill, s.sel, s.inset, s.field] {
                    assert_eq!(c & 0xff, 0xff);
                }
            }
            assert_eq!(super::over(0xffffff00, 0x102030), 0x102030ff);
            assert_eq!(super::over(0xffffffff, 0x102030), 0xffffffff);
        }
    }
}
