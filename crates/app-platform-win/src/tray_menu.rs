//! Menu da bandeja em Liquid Glass (Fase 5): uma janela popup própria no lugar do menu
//! nativo do Win32 (que não tem vidro nem cantos redondos).
//!
//! - Janela sem borda, topmost e sem botão na barra de tarefas; o acrílico do sistema
//!   (`DWMWA_SYSTEMBACKDROP_TYPE` = transient) atrás, cantos `DWMWCP_ROUND`.
//! - Desenho com Direct2D + DirectWrite (Inter embutida, carregada da memória), em pixels
//!   independentes de DPI; cores dos tokens do app (claro/escuro pela barra de tarefas,
//!   sólido com "Efeitos de transparência" desligado ou no Windows 10).
//! - Mesmos itens e checks do menu antigo; linha de status no topo com o ponto vermelho só
//!   gravando; itens de 36 px com raio 10 e vidro no hover; ↑ ↓ Enter Esc; fecha ao perder o
//!   foco. Abre junto do cursor, dentro da área de trabalho do monitor.
//! - Se algo do Direct2D falhar, cai para o menu nativo (o usuário nunca fica sem "Sair").
//!
//! Tudo roda na thread da bandeja (a que tem o loop de mensagens).

use std::cell::RefCell;

use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteFactory5, IDWriteFontCollection,
    IDWriteFontCollection1, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_MEDIUM, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_TRANSIENTWINDOW,
    DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_ESCAPE, VK_RETURN, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetWindowLongPtrW, LoadCursorW, RegisterClassExW, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TrackPopupMenu, CREATESTRUCTW, GWLP_USERDATA, HTCLIENT, HWND_TOPMOST,
    IDC_ARROW, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, SWP_SHOWWINDOW, SW_HIDE,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN, WA_INACTIVE, WM_ACTIVATE, WM_KEYDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT,
    WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_THICKFRAME,
};

use crate::tray::TrayCommand;

/// Um item clicável do menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    pub label: String,
    pub enabled: bool,
    /// `Some` = item com marca (ligado/desligado).
    pub checked: Option<bool>,
    pub command: TrayCommand,
    /// Fio separador antes deste item.
    pub separator_before: bool,
}

/// O que o menu mostra.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuModel {
    /// Linha de status (sem clique), ex.: "Gravando: VALORANT".
    pub status: String,
    pub recording: bool,
    pub entries: Vec<MenuEntry>,
}

// ---------- medidas (DIPs) e cores (tokens do app) ----------

const W: f32 = 272.;
const PAD: f32 = 6.;
const STATUS_H: f32 = 44.;
const ITEM_H: f32 = 36.;
const SEP_H: f32 = 9.;
const ITEM_R: f32 = 10.;
const TEXT_X: f32 = 14.;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Look {
    dark: bool,
    glass: bool,
}

#[derive(Clone, Copy)]
struct Colors {
    /// Véu sobre o acrílico (ou o fundo sólido).
    bg: u32,
    text: u32,
    text2: u32,
    text3: u32,
    hover: u32,
    line: u32,
    red: u32,
    red_halo: u32,
}

impl Look {
    fn detect() -> Self {
        Self {
            dark: !crate::appearance::taskbar_light(),
            glass: crate::appearance::transparency_enabled()
                && !crate::appearance::battery_saver_on(),
        }
    }

    /// Cores em `0xRRGGBBAA` (as do mockup).
    fn colors(self) -> Colors {
        let (bg, solid) = if self.dark {
            (0x14151994, 0x1a1b1fff)
        } else {
            (0xf6f7fa8f, 0xf2f3f6ff)
        };
        if self.dark {
            Colors {
                bg: if self.glass { bg } else { solid },
                text: 0xf5f5f7ff,
                text2: 0xebebf5a3,
                text3: 0xebebf566,
                hover: 0xffffff29,
                line: 0xffffff1a,
                red: 0xff453aff,
                red_halo: 0xff453a2e,
            }
        } else {
            Colors {
                bg: if self.glass { bg } else { solid },
                text: 0x111114ff,
                text2: 0x32323cb8,
                text3: 0x32323c73,
                hover: 0xffffffd9,
                line: 0x00000014,
                red: 0xe5322bff,
                red_halo: 0xe5322b29,
            }
        }
    }
}

fn color(c: u32) -> D2D1_COLOR_F {
    let f = |s: u32| ((c >> s) & 0xff) as f32 / 255.0;
    D2D1_COLOR_F {
        r: f(24),
        g: f(16),
        b: f(8),
        a: f(0),
    }
}

/// Linha do menu já posicionada (em DIPs).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Status { top: f32 },
    Separator { top: f32 },
    Item { index: usize, top: f32 },
}

/// Posição de cada linha e a altura total. Puro (testado).
fn layout(model: &MenuModel) -> (Vec<Row>, f32) {
    let mut rows = vec![Row::Status { top: PAD }];
    let mut y = PAD + STATUS_H;
    for (index, e) in model.entries.iter().enumerate() {
        if e.separator_before {
            rows.push(Row::Separator { top: y });
            y += SEP_H;
        }
        rows.push(Row::Item { index, top: y });
        y += ITEM_H;
    }
    (rows, y + PAD)
}

/// Item sob o ponto (DIPs).
fn hit(model: &MenuModel, x: f32, y: f32) -> Option<usize> {
    if !(PAD..=W - PAD).contains(&x) {
        return None;
    }
    layout(model).0.into_iter().find_map(|r| match r {
        Row::Item { index, top } if (top..top + ITEM_H).contains(&y) => Some(index),
        _ => None,
    })
}

/// Próximo item habilitado andando `step` (±1), dando a volta.
fn step(model: &MenuModel, from: Option<usize>, step: i32) -> Option<usize> {
    let n = model.entries.len() as i32;
    if n == 0 {
        return None;
    }
    let mut i = from.map_or(if step > 0 { -1 } else { n }, |f| f as i32);
    for _ in 0..n {
        i = (i + step).rem_euclid(n);
        if model.entries[i as usize].enabled {
            return Some(i as usize);
        }
    }
    None
}

/// Onde a janela abre: em cima e à esquerda do cursor (como os menus da bandeja),
/// trocando de lado se não couber, sempre dentro da área de trabalho. Puro (testado).
fn place(cursor: (i32, i32), size: (i32, i32), work: (i32, i32, i32, i32)) -> (i32, i32) {
    let (cx, cy) = cursor;
    let (w, h) = size;
    let (l, t, r, b) = work;
    let x = if cx - w >= l { cx - w } else { cx };
    let y = if cy - h >= t { cy - h } else { cy };
    (x.clamp(l, (r - w).max(l)), y.clamp(t, (b - h).max(t)))
}

// ---------- janela ----------

struct Gfx {
    target: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
}

struct Fonts {
    item: IDWriteTextFormat,
    status: IDWriteTextFormat,
    check: IDWriteTextFormat,
}

struct Inner {
    model: MenuModel,
    hover: Option<usize>,
    look: Look,
    /// DPI do monitor onde o menu abriu.
    dpi: u32,
    d2d: ID2D1Factory,
    fonts: Fonts,
    gfx: Option<Gfx>,
    tracking: bool,
    /// Escolhido pelo usuário (lido por quem abriu, depois de fechar).
    on_pick: Box<dyn Fn(TrayCommand)>,
}

/// O menu de vidro da bandeja. Criado uma vez; [`TrayMenu::show`] abre no cursor.
pub struct TrayMenu {
    hwnd: HWND,
    inner: *mut RefCell<Inner>,
}

const CLASS: PCWSTR = w!("GlintTrayMenu");

impl TrayMenu {
    /// Cria a janela (escondida). `on_pick` roda na thread da bandeja.
    pub fn new(on_pick: impl Fn(TrayCommand) + 'static) -> windows::core::Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let fonts = fonts()?;
            let inner = Box::into_raw(Box::new(RefCell::new(Inner {
                model: MenuModel {
                    status: String::new(),
                    recording: false,
                    entries: Vec::new(),
                },
                hover: None,
                look: Look::detect(),
                dpi: 96,
                d2d,
                fonts,
                gfx: None,
                tracking: false,
                on_pick: Box::new(on_pick),
            })));
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: CLASS,
                ..Default::default()
            };
            // Já registrada (segunda bandeja no mesmo processo) também serve.
            let _ = RegisterClassExW(&class);
            let hwnd = match CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS,
                w!(""),
                // WS_THICKFRAME deixa o DWM pôr o acrílico e a borda; a área não-cliente
                // some no WM_NCCALCSIZE.
                WS_POPUP | WS_THICKFRAME,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                Some(inner as *const _),
            ) {
                Ok(h) => h,
                Err(e) => {
                    drop(Box::from_raw(inner));
                    return Err(e);
                }
            };
            Ok(Self { hwnd, inner })
        }
    }

    fn inner(&self) -> &RefCell<Inner> {
        // SAFETY: `inner` vive até o Drop (a janela é destruída antes de soltar).
        unsafe { &*self.inner }
    }

    /// Troca o conteúdo (também com o menu aberto).
    pub fn set_model(&self, model: MenuModel) {
        let mut i = self.inner().borrow_mut();
        if i.model != model {
            i.hover = i.hover.filter(|&h| h < model.entries.len());
            i.model = model;
            drop(i);
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.hwnd), None, false);
            }
        }
    }

    /// Abre o menu junto de `cursor` (pixels físicos da tela).
    pub fn show(&self, cursor: POINT) {
        unsafe {
            let look = Look::detect();
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let mut mi = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut mi);
            let work = mi.rcWork;
            let (w_dip, h_dip) = {
                let mut i = self.inner().borrow_mut();
                i.look = look;
                i.dpi = dx.max(96);
                i.hover = None;
                i.gfx = None;
                (W, layout(&i.model).1)
            };
            let k = dx.max(96) as f32 / 96.0;
            let size = ((w_dip * k).round() as i32, (h_dip * k).round() as i32);
            let (x, y) = place(
                (cursor.x, cursor.y),
                size,
                (work.left, work.top, work.right, work.bottom),
            );
            self.apply_look(look);
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                size.0,
                size.1,
                SWP_SHOWWINDOW,
            );
            // Sem o foco o menu não fecha ao clicar fora.
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// Acrílico, cantos e tema escuro da moldura (ignorado no Windows 10: fica sólido).
    fn apply_look(&self, look: Look) {
        unsafe {
            let dark: i32 = look.dark.into();
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const _ as _,
                4,
            );
            let round = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &round as *const _ as _,
                4,
            );
            let mut glass = look.glass;
            if glass {
                let margins = MARGINS {
                    cxLeftWidth: -1,
                    cxRightWidth: -1,
                    cyTopHeight: -1,
                    cyBottomHeight: -1,
                };
                let backdrop = DWMSBT_TRANSIENTWINDOW;
                glass = DwmExtendFrameIntoClientArea(self.hwnd, &margins).is_ok()
                    && DwmSetWindowAttribute(
                        self.hwnd,
                        DWMWA_SYSTEMBACKDROP_TYPE,
                        &backdrop as *const _ as _,
                        4,
                    )
                    .is_ok();
            }
            self.inner().borrow_mut().look.glass = glass;
        }
    }
}

impl Drop for TrayMenu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            drop(Box::from_raw(self.inner));
        }
    }
}

/// Inter (embutida) para os textos; se o DirectWrite não aceitar a fonte da memória,
/// usa a fonte do sistema.
fn fonts() -> windows::core::Result<Fonts> {
    unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let coll: Option<IDWriteFontCollection> = inter(&factory).ok().and_then(|c| c.cast().ok());
        let family = if coll.is_some() {
            w!("Inter")
        } else {
            w!("Segoe UI")
        };
        let make =
            |weight: DWRITE_FONT_WEIGHT, size: f32| -> windows::core::Result<IDWriteTextFormat> {
                let f = factory.CreateTextFormat(
                    family,
                    coll.as_ref(),
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("pt-BR"),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let trim = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                let sign = factory.CreateEllipsisTrimmingSign(&f)?;
                f.SetTrimming(&trim, &sign)?;
                Ok(f)
            };
        let check = factory.CreateTextFormat(
            w!("Segoe MDL2 Assets"),
            None,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            13.,
            w!("pt-BR"),
        )?;
        check.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        check.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        Ok(Fonts {
            item: make(DWRITE_FONT_WEIGHT_NORMAL, 14.)?,
            status: make(DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.)?,
            check,
        })
    }
}

/// Coleção com a Inter carregada da memória (Windows 10 1709+).
fn inter(factory: &IDWriteFactory) -> windows::core::Result<IDWriteFontCollection1> {
    const FONTS: [&[u8]; 3] = [
        include_bytes!("../../app-ui/assets/fonts/Inter-Regular.ttf"),
        include_bytes!("../../app-ui/assets/fonts/Inter-Medium.ttf"),
        include_bytes!("../../app-ui/assets/fonts/Inter-SemiBold.ttf"),
    ];
    let _ = DWRITE_FONT_WEIGHT_MEDIUM;
    unsafe {
        let f5: IDWriteFactory5 = factory.cast()?;
        let loader = f5.CreateInMemoryFontFileLoader()?;
        f5.RegisterFontFileLoader(&loader)?;
        let builder = f5.CreateFontSetBuilder()?;
        for data in FONTS {
            // Sem dono: o loader copia os bytes.
            let file = loader.CreateInMemoryFontFileReference(
                &f5,
                data.as_ptr() as *const _,
                data.len() as u32,
                None,
            )?;
            builder.AddFontFile(&file)?;
        }
        let set = builder.CreateFontSet()?;
        f5.CreateFontCollectionFromFontSet(&set)
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<Inner>;
        if ptr.is_null() || msg == WM_NCDESTROY {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let cell = &*ptr;
        match msg {
            // Sem moldura: a área cliente é a janela inteira.
            WM_NCCALCSIZE if wp.0 != 0 => LRESULT(0),
            // Sem redimensionar pela borda.
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                if let Ok(mut inner) = cell.try_borrow_mut() {
                    if paint(hwnd, &mut inner).is_err() {
                        inner.gfx = None;
                    }
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = dips(cell, lp);
                let mut inner = cell.borrow_mut();
                let h = hit(&inner.model, x, y).filter(|&i| inner.model.entries[i].enabled);
                if !inner.tracking {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    inner.tracking = TrackMouseEvent(&mut t).is_ok();
                }
                if h != inner.hover {
                    inner.hover = h;
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                let mut inner = cell.borrow_mut();
                inner.tracking = false;
                if inner.hover.take().is_some() {
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = dips(cell, lp);
                let pick = {
                    let inner = cell.borrow();
                    hit(&inner.model, x, y)
                        .filter(|&i| inner.model.entries[i].enabled)
                        .map(|i| inner.model.entries[i].command)
                };
                if let Some(cmd) = pick {
                    choose(hwnd, cell, cmd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wp.0 as u16;
                let mut inner = cell.borrow_mut();
                if key == VK_ESCAPE.0 {
                    drop(inner);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                } else if key == VK_DOWN.0 || key == VK_UP.0 {
                    let dir = if key == VK_DOWN.0 { 1 } else { -1 };
                    inner.hover = step(&inner.model, inner.hover, dir);
                    redraw(hwnd);
                } else if key == VK_RETURN.0 || key == VK_SPACE.0 {
                    let cmd = inner
                        .hover
                        .filter(|&i| inner.model.entries[i].enabled)
                        .map(|i| inner.model.entries[i].command);
                    drop(inner);
                    if let Some(cmd) = cmd {
                        choose(hwnd, cell, cmd);
                    }
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Fecha e manda o comando (depois de esconder, para o menu não ficar por cima).
unsafe fn choose(hwnd: HWND, cell: &RefCell<Inner>, cmd: TrayCommand) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    (cell.borrow().on_pick)(cmd);
}

fn redraw(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

/// Posição do mouse em DIPs.
fn dips(cell: &RefCell<Inner>, lp: LPARAM) -> (f32, f32) {
    let x = (lp.0 & 0xffff) as i16 as f32;
    let y = ((lp.0 >> 16) & 0xffff) as i16 as f32;
    let k = cell.borrow().dpi as f32 / 96.0;
    (x / k, y / k)
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

fn paint(hwnd: HWND, inner: &mut Inner) -> windows::core::Result<()> {
    unsafe {
        if inner.gfx.is_none() {
            let mut rc = RECT::default();
            windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: inner.dpi as f32,
                dpiY: inner.dpi as f32,
                ..Default::default()
            };
            let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U {
                    width: (rc.right - rc.left).max(1) as u32,
                    height: (rc.bottom - rc.top).max(1) as u32,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let target = inner.d2d.CreateHwndRenderTarget(&props, &hprops)?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = target.CreateSolidColorBrush(&color(0xffffffff), None)?;
            inner.gfx = Some(Gfx { target, brush });
        }
        let gfx = inner.gfx.as_ref().expect("criado acima");
        let (t, b) = (&gfx.target, &gfx.brush);
        let c = inner.look.colors();
        let (rows, height) = layout(&inner.model);
        let fill = |col: u32, r: D2D_RECT_F, radius: f32| {
            b.SetColor(&color(col));
            if radius > 0. {
                t.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: r,
                        radiusX: radius,
                        radiusY: radius,
                    },
                    b,
                );
            } else {
                t.FillRectangle(&r, b);
            }
        };
        let text = |s: &str, fmt: &IDWriteTextFormat, col: u32, r: D2D_RECT_F| {
            let wide: Vec<u16> = s.encode_utf16().collect();
            b.SetColor(&color(col));
            t.DrawText(
                &wide,
                fmt,
                &r,
                b,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        };

        t.BeginDraw();
        // Transparente = o acrílico do DWM aparece; o véu do tema por cima.
        t.Clear(Some(&color(if inner.look.glass {
            0x00000000
        } else {
            c.bg
        })));
        if inner.look.glass {
            fill(c.bg, rect(0., 0., W, height), 0.);
        }
        for row in rows {
            match row {
                Row::Status { top } => {
                    let cy = top + STATUS_H / 2.;
                    let cx = PAD + TEXT_X + 4.;
                    let dot = |r: f32, col: u32| {
                        b.SetColor(&color(col));
                        let mut e = D2D1_ELLIPSE {
                            radiusX: r,
                            radiusY: r,
                            ..Default::default()
                        };
                        e.point.X = cx;
                        e.point.Y = cy;
                        t.FillEllipse(&e, b);
                    };
                    if inner.model.recording {
                        dot(8., c.red_halo);
                        dot(4., c.red);
                    } else {
                        dot(4., c.text3);
                    }
                    text(
                        &inner.model.status,
                        &inner.fonts.status,
                        c.text,
                        rect(cx + 14., top, W - PAD - TEXT_X, top + STATUS_H),
                    );
                }
                Row::Separator { top } => {
                    let y = top + SEP_H / 2.;
                    fill(c.line, rect(PAD + 10., y - 0.5, W - PAD - 10., y + 0.5), 0.);
                }
                Row::Item { index, top } => {
                    let e = &inner.model.entries[index];
                    let r = rect(PAD, top, W - PAD, top + ITEM_H);
                    if inner.hover == Some(index) && e.enabled {
                        fill(c.hover, r, ITEM_R);
                    }
                    let col = if e.enabled { c.text } else { c.text3 };
                    let right = if e.checked.is_some() { 40. } else { TEXT_X };
                    text(
                        &e.label,
                        &inner.fonts.item,
                        col,
                        rect(PAD + TEXT_X, top, W - PAD - right, top + ITEM_H),
                    );
                    if e.checked == Some(true) {
                        // U+E73E = CheckMark no Segoe MDL2 Assets.
                        text(
                            "\u{E73E}",
                            &inner.fonts.check,
                            c.text2,
                            rect(W - PAD - 36., top, W - PAD - 8., top + ITEM_H),
                        );
                    }
                }
            }
        }
        t.EndDraw(None, None)
    }
}

/// Menu nativo do Win32, se o de vidro não puder ser criado. Bloqueia até escolher.
pub fn native_menu(owner: HWND, model: &MenuModel, at: POINT) -> Option<TrayCommand> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(Some(0)).collect() };
        let status = wide(&model.status);
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(status.as_ptr()));
        let labels: Vec<Vec<u16>> = model.entries.iter().map(|e| wide(&e.label)).collect();
        for (i, e) in model.entries.iter().enumerate() {
            if e.separator_before {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            }
            let mut flags = MF_STRING;
            if !e.enabled {
                flags |= MF_GRAYED;
            }
            if e.checked == Some(true) {
                flags |= MF_CHECKED;
            }
            let _ = AppendMenuW(menu, flags, i + 1, PCWSTR(labels[i].as_ptr()));
        }
        let _ = SetForegroundWindow(owner);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTALIGN | TPM_BOTTOMALIGN,
            at.x,
            at.y,
            None,
            owner,
            None,
        );
        let _ = DestroyMenu(menu);
        let i = picked.0 as usize;
        (i >= 1)
            .then(|| model.entries.get(i - 1).map(|e| e.command))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> MenuModel {
        let e = |label: &str, enabled, sep, command| MenuEntry {
            label: label.into(),
            enabled,
            checked: None,
            command,
            separator_before: sep,
        };
        MenuModel {
            status: "Esperando jogo".into(),
            recording: false,
            entries: vec![
                e("Salvar", false, true, TrayCommand::SaveClip),
                e("Gravar a tela", true, false, TrayCommand::ToggleManual),
                e("Abrir", true, true, TrayCommand::OpenLibrary),
                e("Sair", true, true, TrayCommand::Quit),
            ],
        }
    }

    #[test]
    fn altura_e_cliques() {
        let m = model();
        let (_, h) = layout(&m);
        assert_eq!(h, PAD + STATUS_H + 3. * SEP_H + 4. * ITEM_H + PAD);
        // Primeiro item logo depois do status e do fio.
        let y0 = PAD + STATUS_H + SEP_H;
        assert_eq!(hit(&m, 100., y0 + 1.), Some(0));
        assert_eq!(hit(&m, 100., y0 + ITEM_H + 1.), Some(1));
        assert_eq!(hit(&m, 100., PAD + 5.), None);
        assert_eq!(hit(&m, 1., y0 + 1.), None);
    }

    #[test]
    fn teclas_pulam_desabilitados() {
        let m = model();
        assert_eq!(step(&m, None, 1), Some(1));
        assert_eq!(step(&m, Some(1), -1), Some(3));
        assert_eq!(step(&m, Some(3), 1), Some(1));
    }

    #[test]
    fn abre_dentro_da_tela() {
        let work = (0, 0, 1920, 1040);
        // Bandeja embaixo à direita: abre em cima e à esquerda do cursor.
        assert_eq!(place((1800, 1060), (272, 400), work), (1528, 640));
        // Cursor no canto de cima à esquerda: abre para baixo e à direita.
        assert_eq!(place((10, 10), (272, 400), work), (10, 10));
        // Maior que a tela: encosta no topo.
        assert_eq!(place((100, 500), (272, 2000), work).1, 0);
    }
}
