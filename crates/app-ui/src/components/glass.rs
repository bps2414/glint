//! Vidro (Liquid Glass neutro, Fase 5): painel, superfície de pílula e chip.
//!
//! O GPUI não tem `backdrop-filter` dentro da janela; o "vidro" é o acrílico da janela
//! atrás + fundo translúcido + realce em gradiente no topo + borda clara + sombra. Onde o
//! vidro fica sobre o vídeo ou miniaturas, o desfoque vem da CPU (`crate::blur`).
//!
//! Hover e pressionado trocam fundo/sombra na hora: o GPUI não tem `transform` em `div`
//! (a escala 0,97 do mockup não existe aqui) e animar o hover exigiria estado por botão.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::theme::glass::Palette;
use crate::theme::size::Size;
use crate::theme::{pal, tok, FONT_NUM, FONT_UI, R_SM};

/// Sombra no estilo do mockup (`0 y blur cor`).
pub fn shadow(y: f32, blur: f32, color: u32) -> BoxShadow {
    BoxShadow {
        color: rgba(color).into(),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }
}

/// Contorno fino por dentro (o `inset 0 0 0 0.5px` do mockup).
pub fn hairline(color: u32) -> BoxShadow {
    BoxShadow {
        color: rgba(color).into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(0.5),
        inset: true,
    }
}

/// Realce do vidro: branco que some de cima para baixo até `until` (0–1) da altura.
pub fn highlight(color: u32, radius: Pixels, until: f32) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(relative(until))
        .rounded_t(radius)
        .bg(linear_gradient(
            180.,
            linear_color_stop(rgba(color), 0.),
            linear_color_stop(rgba(color & 0xffffff00), 1.),
        ))
}

fn shadows(pal: Palette, y: f32, blur: f32, color: u32) -> Vec<BoxShadow> {
    let mut v = vec![shadow(y, blur, color)];
    if pal.panel_outline & 0xff != 0 {
        v.push(hairline(pal.panel_outline));
    }
    v
}

/// Painel flutuante (sidebar, linha do tempo, publicar): raio 20–22.
pub fn panel(pal: Palette, radius: Pixels) -> Div {
    div()
        .relative()
        .rounded(radius)
        .bg(rgba(pal.panel))
        .border_1()
        .border_color(rgba(pal.panel_border))
        .shadow(shadows(pal, 12., 40., pal.panel_shadow))
        .child(highlight(pal.panel_hi, radius, 0.3))
}

/// Painel da paleta ativa.
pub fn glass_panel(radius: Pixels) -> Div {
    panel(pal(), radius)
}

/// Superfície de pílula (fundo, borda clara, sombra e realce) em qualquer elemento
/// arredondado. O elemento define altura e raio.
pub fn pill_surface<E: Styled + ParentElement>(e: E, pal: Palette) -> E {
    let mut e = e
        .bg(rgba(pal.pill))
        .border_1()
        .border_color(rgba(pal.panel_border))
        .shadow(shadows(pal, 6., 20., pal.pill_shadow));
    e.extend([highlight(pal.pill_hi, px(999.), 0.6).into_any_element()]);
    e
}

/// Pílula (campo de busca, ordem): altura padrão, raio cheio.
pub fn pill(pal: Palette) -> Div {
    pill_surface(
        div()
            .relative()
            .flex()
            .items_center()
            .h(Size::Md.h())
            .px(Size::Md.pad())
            .gap(px(8.))
            .rounded_full(),
        pal,
    )
}

/// Superfície de item selecionado (nav, segmentado).
pub fn selected<E: Styled>(e: E, pal: Palette) -> E {
    let mut v = vec![shadow(4., 14., pal.sel_shadow)];
    v.push(hairline(pal.sel_outline));
    e.bg(rgba(pal.sel)).shadow(v)
}

/// Chip de vidro sobre imagem/vídeo (duração, Publicado, favorito). Igual nos dois temas.
pub fn chip() -> Div {
    div()
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(5.))
        .flex_none()
        .h(Size::Xs.h())
        .px(Size::Xs.pad())
        .rounded_full()
        .bg(tok::chip())
        .border_1()
        .border_color(tok::chip_edge())
        .font_family(FONT_UI)
        .text_size(Size::Xs.text())
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(tok::on_media())
}

/// Chip com número (duração): Geist Mono.
pub fn chip_num(text: impl Into<SharedString>) -> Div {
    chip()
        .font_family(FONT_NUM)
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

/// Caixa rebaixada (status do gravador, link publicado).
pub fn inset_box() -> Div {
    let p = pal();
    div()
        .rounded(R_SM)
        .bg(rgba(p.inset))
        .border_1()
        .border_color(tok::line())
        .when(!p.dark, |d| d.border_color(rgba(p.panel_border)))
}

/// Onde uma imagem foi desenhada neste quadro (medido no prepaint, lido no paint).
pub type Area = Rc<Cell<Option<Bounds<Pixels>>>>;

/// O que fica atrás de um vidro sobre imagem: a cópia desfocada e onde ela cobre a tela.
#[derive(Clone)]
pub struct Backdrop {
    pub image: Arc<RenderImage>,
    pub area: Area,
}

/// Mede o pai (posição na janela) em `area`. Colocar como primeiro filho do elemento
/// cuja imagem fica atrás dos chips.
pub fn measure(area: Area) -> impl IntoElement {
    canvas(move |bounds, _, _| area.set(Some(bounds)), |_, _, _, _| {})
        .absolute()
        .inset_0()
}

/// Fundo de vidro fosco: o pedaço da imagem desfocada que fica atrás deste elemento,
/// com o véu `tint` por cima e cantos `radius`. Primeiro filho de um elemento `relative`.
pub fn frosted(backdrop: Backdrop, radius: Pixels, tint: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            if let Some(area) = backdrop.area.get() {
                let _ = window.paint_image(
                    bounds,
                    area,
                    Corners::all(radius),
                    backdrop.image.clone(),
                    0,
                    false,
                );
            }
            window.paint_quad(fill(bounds, tint).corner_radii(radius));
        },
    )
    .absolute()
    .inset_0()
}

/// Chip de vidro fosco sobre imagem (com a cópia desfocada atrás, quando houver).
pub fn chip_on(backdrop: Option<Backdrop>) -> Div {
    match backdrop {
        Some(b) => chip()
            .bg(transparent_black())
            .child(frosted(b, Size::Xs.h() / 2., tok::chip())),
        None => chip(),
    }
}

/// Fundo de vidro com uma imagem desfocada cobrindo o elemento inteiro (`object-fit:
/// cover`), com o véu `tint` por cima. Para o modal (não existe quadro da janela na API).
pub fn frosted_cover(image: Arc<RenderImage>, radius: Pixels, tint: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let s = image.size(0);
            let (iw, ih) = (s.width.0.max(1) as f32, s.height.0.max(1) as f32);
            let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let k = (bw / iw).max(bh / ih);
            let (w, h) = (iw * k, ih * k);
            let cover = Bounds {
                origin: point(
                    bounds.origin.x + px((bw - w) / 2.),
                    bounds.origin.y + px((bh - h) / 2.),
                ),
                size: size(px(w), px(h)),
            };
            let _ =
                window.paint_image(bounds, cover, Corners::all(radius), image.clone(), 0, false);
            window.paint_quad(fill(bounds, tint).corner_radii(radius));
        },
    )
    .absolute()
    .inset_0()
}
