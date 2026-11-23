//! The GTK 4 shell: a native window over the renderer's pixels (ADR 061).
//!
//! Milestone 3 of the Linux shell: the window, a drawing area, and one frame
//! painted from `renderer` into it. `WINTER_SCREENSHOT=<path>` writes the
//! toplevel exactly as GTK draws it — client-side decorations included — and
//! quits, which is how a machine with no display server photographs the shell.
//!
//! Tabs, input and a real pty are later milestones; until they land the screen
//! comes from a scripted connection so the paint path can be exercised.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;

use gtk::cairo;
use gtk::graphene;
use gtk::prelude::*;
use gtk::{Application, ApplicationWindow, DrawingArea, Snapshot, WidgetPaintable, glib};

use renderer::renderer::Renderer;
use renderer::{
    AtlasEngine, CellSizeInDIP, FontDueRasterizer, FontInfo, RenderSettings, Settings, SharedBackend, SoftwareBackend,
};
use server::pty::PtySize;
use utils::{InclusiveRect, Size};

use crate::term_connection::Connection;
use crate::term_control::Control;
use crate::terminal_render::TerminalData;
use crate::view::View;

/// The window opens at this many cells, like the WinUI shell.
const CELLS: (i32, i32) = (100, 28);
/// The face's size in points before the surface scale is applied.
const FONT_SIZE: f32 = 14.0;

/// Starts the GTK shell and runs it until its window closes.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let app = Application::builder().application_id("org.winter.Winter").build();
    app.connect_activate(activate);
    app.run();
    Ok(())
}

fn activate(app: &Application) {
    let screenshot = std::env::var_os("WINTER_SCREENSHOT").map(PathBuf::from);

    let Some(fonts) = fonts(FONT_SIZE, CELLS) else {
        eprintln!("winter: no monospace face found in the usual font directories");
        app.quit();
        return;
    };

    let shared = SharedBackend::new(SoftwareBackend::with_rasterizer(Box::new(fonts.rasterizer)));
    let mut engine = AtlasEngine::with_settings(fonts.settings.clone(), Box::new(shared.clone()));
    engine.set_shaping(Box::new(fonts.shaper));

    let mut renderer = Renderer::new(
        TerminalData::new(fonts.info, RenderSettings::new(), CELLS.0, CELLS.1),
        RenderSettings::new(),
    );
    renderer.add_engine(Box::new(engine));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Winter")
        .default_width(i32::from(fonts.cell.0) * CELLS.0)
        .default_height(i32::from(fonts.cell.1) * CELLS.1)
        .build();

    let area = DrawingArea::new();
    window.set_child(Some(&area));

    let shell = Rc::new(RefCell::new(Shell {
        window: window.clone(),
        renderer,
        shared,
        control: Control::new(Box::new(Scripted::new(SAMPLE)), CELLS),
        view: View::new(),
        cell: fonts.cell,
        grid: CELLS,
        screenshot,
    }));

    {
        let shell = Rc::clone(&shell);
        area.set_draw_func(move |_area, context, width, height| {
            shell.borrow_mut().paint(context, width, height);
        });
    }

    window.present();

    // A toplevel has to be drawn once before it can be snapshotted, so the shot
    // waits for the compositor to have asked for a frame.
    if shell.borrow().screenshot.is_some() {
        let shell = Rc::clone(&shell);
        let app = app.clone();
        glib::timeout_add_local_once(Duration::from_millis(700), move || {
            shell.borrow().write_screenshot(&app);
        });
    }
}

/// The window, the frame it paints and the terminal behind it.
struct Shell {
    window: ApplicationWindow,
    renderer: Renderer<TerminalData>,
    shared: SharedBackend,
    control: Control,
    view: View,
    /// The cell size in physical pixels.
    cell: (u16, u16),
    /// The grid the renderer's target currently holds.
    grid: (i32, i32),
    /// Where `WINTER_SCREENSHOT` asked for a picture, if it did.
    screenshot: Option<PathBuf>,
}

impl Shell {
    /// Fits the grid to the area and paints one frame.
    fn paint(&mut self, context: &cairo::Context, width: i32, height: i32) {
        self.fit(width, height);

        self.control.pump();
        self.renderer
            .data_mut()
            .refresh(&mut self.control, &self.view, true, None);

        let (background, foreground) = self.control.default_colors();
        let _ = self.renderer.set_default_colors(background, foreground);
        if let Ok(table) = self.control.color_table().lock() {
            self.renderer.settings_mut().set_color_table(table.clone());
        }
        self.renderer.invalidate_all();
        if self.renderer.paint_frame().is_err() {
            return;
        }

        // Whatever the whole-cell grid does not cover stays the pane's colour.
        let (red, green, blue) = channels(background);
        context.set_source_rgb(red, green, blue);
        context.paint().ok();

        let (pixels_wide, pixels_high) = (self.shared.width(), self.shared.height());
        if pixels_wide == 0 || pixels_high == 0 {
            return;
        }

        let mut bytes = Vec::with_capacity(4 * pixels_wide as usize * pixels_high as usize);
        self.shared.with_pixels(|pixels| {
            for pixel in pixels.iter().take(pixels_wide as usize * pixels_high as usize) {
                bytes.extend_from_slice(&pixel.to_ne_bytes());
            }
        });

        // `0xAARRGGBB` in a `u32` is BGRA in memory, which is `ARGB32` on a
        // little-endian host -- Cairo's own layout, so no swizzle is needed.
        let Ok(surface) = cairo::ImageSurface::create_for_data(
            bytes,
            cairo::Format::ARgb32,
            pixels_wide as i32,
            pixels_high as i32,
            4 * pixels_wide as i32,
        ) else {
            return;
        };
        if context.set_source_surface(&surface, 0.0, 0.0).is_ok() {
            context.paint().ok();
        }
    }

    /// Makes the terminal and the renderer's target hold whole cells.
    fn fit(&mut self, width: i32, height: i32) {
        let columns = (width / i32::from(self.cell.0)).max(1);
        let rows = (height / i32::from(self.cell.1)).max(1);
        if (columns, rows) == self.grid {
            return;
        }

        self.grid = (columns, rows);
        self.control.resize(columns, rows);
        let target = (
            u16::try_from(i32::from(self.cell.0) * columns).unwrap_or(u16::MAX),
            u16::try_from(i32::from(self.cell.1) * rows).unwrap_or(u16::MAX),
        );
        let _ = self
            .renderer
            .resize(target, InclusiveRect::new(0, 0, columns - 1, rows - 1));
    }

    /// Writes the toplevel as GTK draws it, then quits.
    fn write_screenshot(&self, app: &Application) {
        let Some(path) = self.screenshot.as_deref() else {
            return;
        };

        let paintable = WidgetPaintable::new(Some(&self.window));
        let width = f64::from(self.window.width());
        let height = f64::from(self.window.height());
        let snapshot = Snapshot::new();
        paintable.snapshot(&snapshot, width, height);

        let Some(node) = snapshot.to_node() else {
            eprintln!("winter: the window produced no render node");
            app.quit();
            return;
        };
        let Some(native) = self.window.native() else {
            eprintln!("winter: the window has no native surface");
            app.quit();
            return;
        };

        let viewport = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
        let Some(renderer) = native.renderer() else {
            eprintln!("winter: the surface has no renderer");
            app.quit();
            return;
        };
        let texture = renderer.render_texture(&node, Some(&viewport));

        match texture.save_to_png(path) {
            Ok(()) => println!("winter: wrote {}", path.display()),
            Err(error) => eprintln!("winter: cannot write {}: {error}", path.display()),
        }
        app.quit();
    }
}

/// A connection that delivers one scripted screen and then waits.
///
/// Milestone 4 replaces this with `Session` over a pty; until then it keeps the
/// paint path honest without a shell behind it.
struct Scripted {
    output: Option<Vec<u8>>,
}

impl Scripted {
    fn new(bytes: &[u8]) -> Self {
        Self {
            output: Some(bytes.to_vec()),
        }
    }
}

impl Connection for Scripted {
    fn drain_into(&mut self, out: &mut Vec<u8>) -> bool {
        out.extend_from_slice(&self.output.take().unwrap_or_default());
        false
    }

    fn write(&mut self, _bytes: &[u8]) -> io::Result<()> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> io::Result<()> {
        Ok(())
    }

    fn exited(&mut self) -> bool {
        false
    }

    fn close(&mut self) {}
}

/// The face, its metrics and the two rasterisers a renderer wants.
struct Fonts {
    info: FontInfo,
    settings: Settings,
    shaper: FontDueRasterizer,
    rasterizer: FontDueRasterizer,
    cell: (u16, u16),
}

/// Finds the host's monospace face and measures it at `size`.
fn fonts(size: f32, cells: (i32, i32)) -> Option<Fonts> {
    let directories = crate::app::font_directories();
    if directories.is_empty() {
        return None;
    }

    let paths: Vec<&Path> = directories.iter().map(PathBuf::as_path).collect();
    let collection = renderer::FontCollection::from_directories(paths);
    let face = collection.default_monospace(renderer::PREFERRED_MONOSPACE_FAMILIES)?;

    let shaper = FontDueRasterizer::from_file(face.path(), size).ok()?;
    let rasterizer = FontDueRasterizer::from_file(face.path(), size).ok()?;

    let advance = shaper.advance_width('M');
    let (ascent, descent) = shaper.vertical_metrics();
    let cell_w = (advance.ceil() as u16).max(1);
    // The cell is the face's own box, so a glyph cannot spill into the row
    // above -- which on row zero means it cannot be clipped by the widget.
    let baseline = (ascent.ceil() as u16).max(1);
    let descender = (descent.ceil() as u16).max(1);
    let cell_h = baseline.saturating_add(descender);

    let mut info = FontInfo::default();
    info.set_from_engine(
        face.family().to_string(),
        0,
        400,
        65001,
        size * 0.75,
        CellSizeInDIP {
            width: f32::from(cell_w),
            height: f32::from(cell_h),
        },
        Size::new(i32::from(cell_w), i32::from(cell_h)),
    );

    let target = |cells: i32, cell: u16| {
        cell.saturating_mul(u16::try_from(cells.max(1)).unwrap_or(u16::MAX))
            .max(1)
    };
    let settings = Settings {
        font: renderer::FontSettings {
            font_name: face.family().to_string(),
            font_size: size,
            cell_size: (cell_w, cell_h),
            font_weight: 400,
            advance_width: cell_w,
            baseline,
            descender,
            thin_line_width: 1,
            dpi: 96,
            ..renderer::FontSettings::default()
        },
        misc: renderer::MiscellaneousSettings {
            background_color: 0x000C_0C0C,
            foreground_color: 0x00CC_CCCC,
            ..renderer::MiscellaneousSettings::default()
        },
        target_size: (target(cells.0, cell_w), target(cells.1, cell_h)),
        viewport_cell_count: (
            u16::try_from(cells.0.max(1)).unwrap_or(u16::MAX),
            u16::try_from(cells.1.max(1)).unwrap_or(u16::MAX),
        ),
        ..Settings::default()
    };

    Some(Fonts {
        info,
        settings,
        shaper,
        rasterizer,
        cell: (cell_w, cell_h),
    })
}

/// A `COLORREF` as Cairo's three components.
fn channels(color: u32) -> (f64, f64, f64) {
    let red = f64::from((color >> 16) & 0xFF) / 255.0;
    let green = f64::from((color >> 8) & 0xFF) / 255.0;
    let blue = f64::from(color & 0xFF) / 255.0;
    (red, green, blue)
}

/// What the skeleton paints: enough to prove the render path end to end.
const SAMPLE: &[u8] = b"\x1b[1;36mWinter\x1b[0m \x1b[2m-\x1b[0m GTK 4 shell, milestone 3\r\n\
\r\n\
This frame is \x1b[1mrenderer\x1b[0m pixels blitted into a GtkDrawingArea.\r\n\
Tabs, input and a real pty are the next milestones.\r\n\
\r\n\
\x1b[33mnormal\x1b[0m  \x1b[1;33mbold\x1b[0m  \x1b[2mdim\x1b[0m  \x1b[3mitalic\x1b[0m  \x1b[4munderline\x1b[0m  \x1b[7mreverse\x1b[0m\r\n\
\x1b[38;5;203mred\x1b[0m  \x1b[38;5;114mgreen\x1b[0m  \x1b[38;5;75mblue\x1b[0m  \x1b[38;5;221myellow\x1b[0m\r\n\
\r\n\
\x1b[32m$\x1b[0m echo \"hello from the renderer\"\r\n\
hello from the renderer\r\n\
\x1b[32m$\x1b[0m ";
