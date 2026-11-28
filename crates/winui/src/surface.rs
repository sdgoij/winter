//! The D3D11/Direct2D surface behind a `SwapChainPanel`.
//!
//! `windows-reactor` hands the panel's `ISwapChainPanelNative` the swap chain
//! we create here; this module only has to produce an `IDXGISwapChain` and blit
//! the software renderer's pixels into it. Every `unsafe` call carries a
//! `SAFETY:` note (ADR 041).

use std::mem::ManuallyDrop;
use std::ptr::NonNull;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1Bitmap, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1Image, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectWrite::{DWRITE_GLYPH_RUN, DWRITE_MEASURING_MODE_NATURAL, IDWriteFontFace};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MATRIX_3X2_F, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGIOutput,
    IDXGISurface, IDXGISwapChain1, IDXGISwapChain2,
};
use windows::core::{BOOL, Interface as _, Result};
use windows_numerics::Vector2;

/// A composition swap chain and the Direct2D context that draws into it.
pub struct Surface {
    // The interfaces the context was made from stay alive with it, but keeping
    // them here is what the COM reference count expects.
    _device: ID3D11Device,
    _dxgi: IDXGIDevice,
    _d2d_device: ID2D1Device,
    swap_chain: IDXGISwapChain1,
    context: ID2D1DeviceContext,
    _target: Option<ID2D1Bitmap1>,
    width: u32,
    height: u32,
    /// Whether a composite frame is open, so a pane's backend must not begin or
    /// present one of its own.
    composite: bool,
}

// The target is held so the context's target stays alive; nothing reads it.

impl Surface {
    /// Creates a surface of `width` by `height` device pixels.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let mut device = None;
        // SAFETY: the pointers are either null (allowed) or point at locals
        // that outlive the call; `D3D11CreateDevice` fills them on success.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )?;
        }
        let device: ID3D11Device = device.ok_or_else(windows::core::Error::empty)?;

        // SAFETY: `device` implements `IDXGIDevice` (BGRA support was requested)
        // and the adapter and factory are alive for the calls that use them.
        let dxgi: IDXGIDevice = device.cast()?;
        let adapter = unsafe { dxgi.GetAdapter()? };
        // SAFETY: the adapter is a child of the factory that created it.
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent()? };

        let description = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: BOOL(0),
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: 0,
        };
        // SAFETY: the description is a valid stack value and the device is a
        // live D3D11 device.
        let swap_chain = unsafe { factory.CreateSwapChainForComposition(&device, &description, None::<&IDXGIOutput>)? };

        // SAFETY: the factory is single-threaded and stays alive in this scope.
        let d2d_factory: ID2D1Factory1 = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        // SAFETY: the DXGI device is a live D3D11-backed device.
        let d2d_device = unsafe { d2d_factory.CreateDevice(&dxgi)? };
        // SAFETY: the options value is a defined constant.
        let context = unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)? };

        let target = create_target(&context, &swap_chain)?;

        Ok(Self {
            _device: device,
            _dxgi: dxgi,
            _d2d_device: d2d_device,
            swap_chain,
            context,
            _target: Some(target),
            width,
            height,
            composite: false,
        })
    }

    /// Resizes the swap chain to `width` by `height` device pixels.
    ///
    /// Every reference to a back buffer must be released before `ResizeBuffers`,
    /// so the Direct2D target is dropped and recreated around the call.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if (self.width, self.height) == (width, height) || width == 0 || height == 0 {
            return Ok(());
        }

        // SAFETY: the context is live; releasing its target satisfies
        // `ResizeBuffers`' requirement that no back buffer be referenced.
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self._target = None;

        // SAFETY: the swap chain is a live composition swap chain, a buffer
        // count of zero keeps the existing count, and the format is unchanged.
        unsafe {
            self.swap_chain
                .ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        }

        self._target = Some(create_target(&self.context, &self.swap_chain)?);
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Undoes the panel's composition scale.
    ///
    /// A `SwapChainPanel` multiplies its swap chain's content by the composition
    /// scale; sizing the swap chain in device pixels and setting the inverse
    /// transform keeps one device pixel on one physical pixel.
    pub fn set_scale(&mut self, scale_x: f32, scale_y: f32) -> Result<()> {
        if !(scale_x > 0.0 && scale_y > 0.0 && scale_x.is_finite() && scale_y.is_finite()) {
            return Ok(());
        }

        let transform = DXGI_MATRIX_3X2_F {
            _11: 1.0 / scale_x,
            _12: 0.0,
            _21: 0.0,
            _22: 1.0 / scale_y,
            _31: 0.0,
            _32: 0.0,
        };
        let chain: IDXGISwapChain2 = self.swap_chain.cast()?;

        // SAFETY: a composition swap chain accepts a transform, and the matrix
        // is a valid scale matrix on the stack for the duration of the call.
        unsafe { chain.SetMatrixTransform(&transform) }
    }

    /// Returns the swap chain as the interface `windows-reactor` attaches.
    pub fn swap_chain(&self) -> windows_core::IUnknown {
        let raw = self.swap_chain.as_raw();
        // SAFETY: a live COM interface is never null; `to_owned` calls `AddRef`,
        // so the returned handle owns a reference of its own.
        let borrowed = unsafe {
            windows_core::InterfaceRef::<windows_core::IUnknown>::from_raw(
                NonNull::new(raw).expect("a swap chain is non-null"),
            )
        };
        borrowed.to_owned()
    }

    /// Whether a composite frame is open.
    pub fn is_composite(&self) -> bool {
        self.composite
    }

    /// Begins one frame that several panes draw into.
    ///
    /// The whole target is cleared to `background` once; each pane then draws
    /// its own grid at its own offset until [`Self::end_composite`].
    pub fn begin_composite(&mut self, background: u32) -> Result<()> {
        if self.composite {
            return Ok(());
        }

        // SAFETY: the context is live and the swap chain is a composition swap
        // chain, so buffer zero is the current back buffer.
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self._target = Some(create_target(&self.context, &self.swap_chain)?);

        let clear = D2D1_COLOR_F {
            r: f32::from((background & 0xFF) as u8) / 255.0,
            g: f32::from(((background >> 8) & 0xFF) as u8) / 255.0,
            b: f32::from(((background >> 16) & 0xFF) as u8) / 255.0,
            a: 1.0,
        };

        // SAFETY: BeginDraw pairs with `end_composite`'s EndDraw on every path.
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&clear as *const _));
        }
        self.composite = true;
        Ok(())
    }

    /// Ends a composite frame and presents it.
    pub fn end_composite(&mut self) -> Result<()> {
        if !self.composite {
            return Ok(());
        }
        self.composite = false;

        // SAFETY: `begin_composite` called BeginDraw on this context, and the
        // swap chain is live.
        unsafe {
            if let Err(error) = self.context.EndDraw(None, None) {
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| eprintln!("winter gpu: EndDraw: {error}"));
                return Err(error);
            }
            let hr = self.swap_chain.Present(1, DXGI_PRESENT(0));
            if hr.is_err() {
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| eprintln!("winter gpu: Present: {hr:?}"));
                return hr.ok();
            }
        }
        Ok(())
    }

    /// Draws a pane's software pixels at `(x, y)` inside a composite frame.
    pub fn blit_at(&mut self, pixels: &[u32], width: u32, height: u32, x: i32, y: i32) -> Result<()> {
        if !self.composite || width == 0 || height == 0 || pixels.len() < (width as usize) * (height as usize) {
            return Ok(());
        }

        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS(0),
            colorContext: ManuallyDrop::new(None),
        };
        let size = D2D_SIZE_U { width, height };

        // SAFETY: the pixel buffer is `width * height` `u32`s, the pitch matches
        // the row length, and the properties describe that layout.
        let bitmap: ID2D1Bitmap1 = unsafe {
            self.context
                .CreateBitmap(size, Some(pixels.as_ptr().cast()), width * 4, &properties as *const _)?
        };
        let bitmap: ID2D1Bitmap = bitmap.into();
        let destination = D2D_RECT_F {
            left: x as f32,
            top: y as f32,
            right: (x + width as i32) as f32,
            bottom: (y + height as i32) as f32,
        };

        // SAFETY: a composite frame is open, and the bitmap belongs to this
        // context's device.
        unsafe {
            self.context.DrawBitmap(
                &bitmap,
                Some(&destination),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        Ok(())
    }

    /// Begins a frame: re-acquires the back buffer, begins drawing and clears.
    ///
    /// A flip-model swap chain rotates its buffers on `Present`, so the target is
    /// re-acquired from the new back buffer each frame; drawing into the old one
    /// leaves a frame on screen that was never painted.
    pub fn begin_frame(&mut self, background: u32) -> Result<()> {
        // SAFETY: the context is live and the swap chain is a composition swap
        // chain, so buffer zero is the current back buffer.
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self._target = Some(create_target(&self.context, &self.swap_chain)?);

        let clear = D2D1_COLOR_F {
            r: f32::from((background & 0xFF) as u8) / 255.0,
            g: f32::from(((background >> 8) & 0xFF) as u8) / 255.0,
            b: f32::from(((background >> 16) & 0xFF) as u8) / 255.0,
            a: 1.0,
        };

        // SAFETY: BeginDraw pairs with `end_frame`'s EndDraw on every path.
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&clear as *const _));
        }
        Ok(())
    }

    /// Ends the frame and presents the back buffer.
    pub fn end_frame(&mut self) -> Result<()> {
        // SAFETY: `begin_frame` called BeginDraw on this context, and the swap
        // chain is live.
        unsafe {
            if let Err(error) = self.context.EndDraw(None, None) {
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| eprintln!("winter gpu: EndDraw: {error}"));
                return Err(error);
            }
            let hr = self.swap_chain.Present(1, DXGI_PRESENT(0));
            if hr.is_err() {
                static ONCE: std::sync::Once = std::sync::Once::new();
                ONCE.call_once(|| eprintln!("winter gpu: Present: {hr:?}"));
                return hr.ok();
            }
        }
        Ok(())
    }

    /// Creates a CPU-writable bitmap for the glyph atlas or an image.
    pub fn create_bitmap(
        &self,
        width: u32,
        height: u32,
        pixels: &[u32],
        alpha_mode: D2D1_ALPHA_MODE,
    ) -> Result<ID2D1Bitmap1> {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: alpha_mode,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS(0),
            colorContext: ManuallyDrop::new(None),
        };
        let size = D2D_SIZE_U { width, height };

        // SAFETY: the pixel buffer is `width * height` `u32`s, the pitch matches
        // the row length, and the properties describe that layout.
        unsafe {
            self.context
                .CreateBitmap(size, Some(pixels.as_ptr().cast()), width * 4, &properties as *const _)
        }
    }

    /// Creates a solid brush whose colour a frame changes per draw.
    pub fn create_solid_brush(&self) -> Result<ID2D1SolidColorBrush> {
        let color = D2D1_COLOR_F {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        // SAFETY: the colour is a stack value and no brush properties are asked
        // for, so Direct2D uses the identity transform.
        unsafe { self.context.CreateSolidColorBrush(&color, None) }
    }

    /// Sets a brush's colour.
    pub fn set_brush_color(&mut self, brush: &ID2D1SolidColorBrush, color: D2D1_COLOR_F) {
        // SAFETY: the brush is a live solid-colour brush from this context.
        unsafe { brush.SetColor(&color) };
    }

    /// Fills `destination` with `brush`'s colour.
    pub fn fill_rect(&mut self, brush: &ID2D1SolidColorBrush, destination: D2D_RECT_F) {
        // SAFETY: the brush belongs to this device and the rectangle is a value
        // on the stack for the duration of the call.
        unsafe { self.context.FillRectangle(&destination, brush) };
    }

    /// Draws one run of glyphs, tinted by `brush`.
    pub fn draw_glyph_run(
        &mut self,
        brush: &ID2D1SolidColorBrush,
        origin: Vector2,
        face: &IDWriteFontFace,
        em_size: f32,
        glyphs: &[u16],
        advances: &[f32],
    ) -> Result<()> {
        if glyphs.is_empty() || advances.len() != glyphs.len() {
            return Ok(());
        }
        let run = DWRITE_GLYPH_RUN {
            fontFace: ManuallyDrop::new(Some(face.clone())),
            fontEmSize: em_size,
            glyphCount: glyphs.len() as u32,
            glyphIndices: glyphs.as_ptr(),
            glyphAdvances: advances.as_ptr(),
            glyphOffsets: std::ptr::null(),
            isSideways: BOOL(0),
            bidiLevel: 0,
        };
        // SAFETY: the run borrows the two slices for the duration of the call,
        // and the brush belongs to this context's device.
        unsafe {
            self.context
                .DrawGlyphRun(origin, &run, None, brush, DWRITE_MEASURING_MODE_NATURAL);
        }
        Ok(())
    }

    /// Draws text with grayscale antialiasing, matching the software backend.
    pub fn set_grayscale_text(&mut self) {
        // SAFETY: the context is live.
        unsafe {
            self.context.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }
    }

    /// Draws one bitmap into `destination`, one pixel to one pixel.
    pub fn draw_bitmap(&mut self, bitmap: &ID2D1Bitmap1, destination: D2D_RECT_F) -> Result<()> {
        let bitmap: &ID2D1Bitmap = bitmap;
        // SAFETY: the bitmap belongs to this context's device and the rectangle
        // is a value on the stack for the duration of the call.
        unsafe {
            self.context.DrawBitmap(
                bitmap,
                Some(&destination),
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        Ok(())
    }

    /// Blits `pixels` (`0xAARRGGBB`, which is BGRA in memory) and presents.
    ///
    /// The grid is drawn at the top left, one pixel to one pixel; the rest of
    /// the surface is cleared to `background` (a `COLORREF`).
    pub fn present(&mut self, pixels: &[u32], width: u32, height: u32, background: u32) -> Result<()> {
        if width == 0 || height == 0 || pixels.len() < (width as usize) * (height as usize) {
            return Ok(());
        }

        // A flip-model swap chain rotates its buffers on `Present`, so the target
        // is re-acquired from the new back buffer each frame; drawing into the
        // old one leaves a frame on screen that was never painted.
        // SAFETY: the context is live and the swap chain is a composition swap
        // chain, so buffer zero is the current back buffer.
        unsafe { self.context.SetTarget(None::<&ID2D1Image>) };
        self._target = Some(create_target(&self.context, &self.swap_chain)?);

        let clear = D2D1_COLOR_F {
            r: f32::from((background & 0xFF) as u8) / 255.0,
            g: f32::from(((background >> 8) & 0xFF) as u8) / 255.0,
            b: f32::from(((background >> 16) & 0xFF) as u8) / 255.0,
            a: 1.0,
        };

        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS(0),
            colorContext: ManuallyDrop::new(None),
        };
        let size = D2D_SIZE_U { width, height };

        // SAFETY: the pixel buffer is `width * height` `u32`s, the pitch matches
        // the row length, and the properties describe that layout.
        let bitmap: ID2D1Bitmap1 = unsafe {
            self.context
                .CreateBitmap(size, Some(pixels.as_ptr().cast()), width * 4, &properties as *const _)?
        };
        let bitmap: ID2D1Bitmap = bitmap.into();

        // SAFETY: the target was set in `new` (or `resize`); Direct2D requires
        // BeginDraw and EndDraw to pair, which they do on every path out of this
        // block. The clear colour outlives the call and the bitmap is drawn at
        // the origin, one pixel to one pixel.
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(Some(&clear as *const _));
            self.context
                .DrawBitmap(&bitmap, None, 1.0, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, None, None);
            self.context.EndDraw(None, None)?;
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
        }

        Ok(())
    }
}

/// Creates the back-buffer bitmap and makes it the context's target.
fn create_target(context: &ID2D1DeviceContext, swap_chain: &IDXGISwapChain1) -> Result<ID2D1Bitmap1> {
    // SAFETY: buffer zero is the back buffer of a flip-model swap chain.
    let surface: IDXGISurface = unsafe { swap_chain.GetBuffer(0)? };
    let properties = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
        colorContext: ManuallyDrop::new(None),
    };
    // SAFETY: the surface is a live DXGI surface with the format above.
    let target = unsafe { context.CreateBitmapFromDxgiSurface(&surface, Some(&properties as *const _))? };
    // SAFETY: the bitmap is a target bitmap created from the context's device.
    unsafe { context.SetTarget(&target) };
    Ok(target)
}
