//! The D3D11/Direct2D surface behind a `SwapChainPanel`.
//!
//! `windows-reactor` hands the panel's `ISwapChainPanelNative` the swap chain
//! we create here; this module only has to produce an `IDXGISwapChain` and blit
//! the software renderer's pixels into it. Every `unsafe` call carries a
//! `SAFETY:` note (ADR 041).

use std::mem::ManuallyDrop;
use std::ptr::NonNull;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1CreateFactory, ID2D1Bitmap, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGIOutput, IDXGISurface, IDXGISwapChain1,
};
use windows::core::{BOOL, Interface as _, Result};

/// A composition swap chain and the Direct2D context that draws into it.
pub struct Surface {
    // The interfaces the context was made from stay alive with it, but keeping
    // them here is what the COM reference count expects.
    _device: ID3D11Device,
    _dxgi: IDXGIDevice,
    _d2d_device: ID2D1Device,
    swap_chain: IDXGISwapChain1,
    context: ID2D1DeviceContext,
    _target: ID2D1Bitmap1,
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
            _target: target,
        })
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

    /// Blits `pixels` (`0xAARRGGBB`, which is BGRA in memory) and presents.
    pub fn present(&mut self, pixels: &[u32], width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 || pixels.len() < (width as usize) * (height as usize) {
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

        // SAFETY: the target was set in `new`; Direct2D requires BeginDraw and
        // EndDraw to pair, which they do on every path out of this block.
        unsafe {
            self.context.BeginDraw();
            self.context.Clear(None);
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
