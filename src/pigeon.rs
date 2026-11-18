#[cfg(not(feature = "standalone"))]
use std::path::PathBuf;
#[cfg(any(not(feature = "standalone"), test))]
use std::{os::windows::ffi::OsStrExt, path::Path};
#[cfg(any(not(feature = "standalone"), test))]
use windows::{Win32::Foundation::GENERIC_READ, core::PCWSTR};
use windows::{
    Win32::{
        Foundation::{E_FAIL, E_INVALIDARG},
        Graphics::{
            Direct2D::{
                Common::{
                    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
                },
                D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR, ID2D1Bitmap1,
                ID2D1DeviceContext,
            },
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapDecoder,
                IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
                WICDecodeMetadataCacheOnLoad,
            },
        },
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
    },
    core::{Error, Result},
};
use windows_numerics::Matrix3x2;

mod hit_region;
#[cfg(feature = "standalone")]
mod packed;

const FRAME_WIDTH: u32 = 512;
const FRAME_HEIGHT: u32 = 408;
const FRAME_PITCH: u32 = FRAME_WIDTH * 4;
const FRAME_BYTES: usize = (FRAME_PITCH * FRAME_HEIGHT) as usize;
pub(crate) const GROUND_Y: f32 = 72.0;
// The 2x image covers native coordinates (24,40)-(280,244) around the (160,145) anchor.
const DESTINATION: D2D_RECT_F = D2D_RECT_F {
    left: -136.0,
    top: -105.0,
    right: 120.0,
    bottom: 99.0,
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpriteView {
    frame: (&'static str, u32),
    facing: f32,
    scale: f32,
}

impl SpriteView {
    pub fn new((directory, phase): (&'static str, f32), facing: f32, scale: f32) -> Self {
        let count = crate::motion::sprite_frame_count(directory, cfg!(feature = "painted"));
        Self {
            frame: (directory, frame_index(phase, count)),
            facing,
            scale,
        }
    }
}

pub struct PigeonRenderer {
    factory: IWICImagingFactory,
    #[cfg(not(feature = "standalone"))]
    assets: PathBuf,
    #[cfg(feature = "standalone")]
    assets: packed::PackedFrames,
    bitmap: ID2D1Bitmap1,
    pixels: Vec<u8>,
    last_frame: Option<(&'static str, u32)>,
}

impl PigeonRenderer {
    pub(crate) fn frame_pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn new(context: &ID2D1DeviceContext) -> Result<Self> {
        let executable = std::env::current_exe()
            .map_err(|error| Error::new(E_FAIL, format!("Cannot locate pigeon assets: {error}")))?;
        #[cfg(not(feature = "standalone"))]
        let assets = executable
            .parent()
            .ok_or_else(|| Error::new(E_FAIL, "The executable has no parent directory"))?
            .join("assets");
        #[cfg(all(debug_assertions, feature = "painted", not(feature = "standalone")))]
        let assets = if assets.is_dir() {
            assets
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets-2d")
        };
        #[cfg(feature = "standalone")]
        let assets = packed::PackedFrames::open(&executable)?;
        let pixels = vec![0; FRAME_BYTES];
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        Ok(Self {
            factory: unsafe {
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?
            },
            assets,
            bitmap: unsafe {
                context.CreateBitmap(
                    D2D_SIZE_U {
                        width: FRAME_WIDTH,
                        height: FRAME_HEIGHT,
                    },
                    Some(pixels.as_ptr().cast()),
                    FRAME_PITCH,
                    &properties,
                )?
            },
            pixels,
            last_frame: None,
        })
    }

    pub fn draw(
        &mut self,
        context: &ID2D1DeviceContext,
        view: SpriteView,
        origin_x: f32,
        origin_y: f32,
    ) -> Result<()> {
        let (directory, index) = view.frame;
        if self.last_frame != Some(view.frame) {
            // Keep one lossless frame on the CPU and GPU; repeated presentation skips decoding.
            #[cfg(not(feature = "standalone"))]
            decode_frame(
                &self.factory,
                &self.assets.join(directory).join(format!("{index:04}.png")),
                &mut self.pixels,
            )?;
            #[cfg(feature = "standalone")]
            decode_memory(
                &self.factory,
                self.assets.frame(directory, index)?,
                &mut self.pixels,
            )?;
            unsafe {
                self.bitmap
                    .CopyFromMemory(None, self.pixels.as_ptr().cast(), FRAME_PITCH)?;
            }
            self.last_frame = Some(view.frame);
        }
        unsafe {
            let mut saved = Matrix3x2::default();
            context.GetTransform(&mut saved);
            context.SetTransform(&Matrix3x2 {
                M11: view.scale * view.facing,
                M22: view.scale,
                M31: origin_x,
                M32: origin_y,
                ..Default::default()
            });
            context.DrawBitmap(
                &self.bitmap,
                Some(&DESTINATION),
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
            context.SetTransform(&saved);
        }
        Ok(())
    }
}

#[cfg(any(not(feature = "standalone"), test))]
fn decode_frame(factory: &IWICImagingFactory, path: &Path, pixels: &mut [u8]) -> Result<()> {
    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(wide_path.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnLoad,
            )
            .map_err(|error| {
                Error::new(
                    error.code(),
                    format!("Cannot load {}: {}", path.display(), error.message()),
                )
            })?;
        decode_pixels(factory, &decoder, pixels)
    }
}

#[cfg(feature = "standalone")]
fn decode_memory(factory: &IWICImagingFactory, bytes: &[u8], pixels: &mut [u8]) -> Result<()> {
    unsafe {
        let stream = factory.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder: IWICBitmapDecoder = CoCreateInstance(
            &windows::Win32::Graphics::Imaging::CLSID_WICPngDecoder,
            None,
            CLSCTX_INPROC_SERVER,
        )?;
        decoder.Initialize(&stream, WICDecodeMetadataCacheOnLoad)?;
        decode_pixels(factory, &decoder, pixels)
    }
}

fn decode_pixels(
    factory: &IWICImagingFactory,
    decoder: &IWICBitmapDecoder,
    pixels: &mut [u8],
) -> Result<()> {
    unsafe {
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        if (width, height) != (FRAME_WIDTH, FRAME_HEIGHT) || pixels.len() != FRAME_BYTES {
            return Err(Error::new(
                E_INVALIDARG,
                format!(
                    "Invalid frame: expected {FRAME_WIDTH}x{FRAME_HEIGHT}, found {width}x{height}"
                ),
            ));
        }
        let converter = factory.CreateFormatConverter()?;
        // PNG is straight RGBA; WIC performs the sole premultiplication for Direct2D.
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        converter.CopyPixels(std::ptr::null(), FRAME_PITCH, pixels)
    }
}

#[cfg(feature = "standalone")]
pub(crate) fn verify_embedded_assets() -> Result<()> {
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
    }
    let check = || -> Result<()> {
        let mut assets = packed::PackedFrames::open(&std::env::current_exe()?)?;
        let factory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
        let mut pixels = vec![0; FRAME_BYTES];
        for clip in packed::CLIPS {
            for index in 0..crate::motion::sprite_frame_count(clip, true) {
                decode_memory(&factory, assets.frame(clip, index)?, &mut pixels).map_err(
                    |error| {
                        Error::new(
                            error.code(),
                            format!("{clip}/{index:04}: {}", error.message()),
                        )
                    },
                )?;
            }
        }
        Ok(())
    };
    let result = check();
    unsafe {
        CoUninitialize();
    }
    result
}

fn frame_index(phase: f32, count: u32) -> u32 {
    let phase = if phase.is_finite() {
        phase.rem_euclid(1.0)
    } else {
        0.0
    };
    ((phase * count as f32) as u32).min(count - 1)
}

#[cfg(test)]
mod tests {
    use super::{FRAME_BYTES, FRAME_HEIGHT, FRAME_WIDTH, SpriteView, decode_frame, frame_index};
    use std::path::Path;
    use windows::Win32::{
        Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
    };

    #[test]
    fn only_identical_sprite_views_can_skip_presentation() {
        let view = SpriteView::new(("look", 0.0), 1.0, 1.15);
        assert_eq!(view, SpriteView::new(("look", 0.0005), 1.0, 1.15));
        assert_ne!(view, SpriteView::new(("look", 0.02), 1.0, 1.15));
        assert_ne!(view, SpriteView::new(("peck", 0.0), 1.0, 1.15));
        assert_ne!(view, SpriteView::new(("look", 0.0), -1.0, 1.15));
        assert_ne!(view, SpriteView::new(("look", 0.0), 1.0, 1.5));
        assert_ne!(
            SpriteView::new(("walk", 0.5), 1.0, 1.0),
            SpriteView::new(("stalk", 0.5), 1.0, 1.0)
        );
        assert_eq!(
            SpriteView::new(("settle", 0.99), 1.0, 1.0).frame,
            (
                "settle",
                crate::motion::sprite_frame_count("settle", cfg!(feature = "painted")) - 1
            )
        );
    }

    #[cfg(not(feature = "painted"))]
    #[test]
    fn lossless_frames_decode_to_premultiplied_pixels_with_transparent_borders() {
        check_lossless_frames("assets");
    }

    #[test]
    fn painted_frames_decode_with_the_desktop_decoder() {
        check_lossless_frames("assets-2d");
    }

    fn check_lossless_frames(assets: &str) {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let mut pixels = vec![0; FRAME_BYTES];
            let painted = assets == "assets-2d";
            let mut clips = vec!["walk", "look", "peck"];
            if painted {
                clips.extend(["stalk", "settle", "turn"]);
            }
            for action in clips {
                let count = crate::motion::sprite_frame_count(action, painted) as usize;
                let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(assets)
                    .join(action);
                assert_eq!(
                    std::fs::read_dir(&directory)
                        .unwrap()
                        .filter_map(|entry| entry.ok())
                        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
                        .count(),
                    count
                );
                for index in 0..count {
                    let path = directory.join(format!("{index:04}.png"));
                    decode_frame(&factory, &path, &mut pixels).unwrap();
                    #[cfg(feature = "standalone")]
                    if index == 0 || index == count - 1 {
                        let mut packed = vec![0; FRAME_BYTES];
                        super::decode_memory(&factory, &std::fs::read(&path).unwrap(), &mut packed)
                            .unwrap();
                        assert!(
                            packed == pixels,
                            "{} changed when decoded from memory",
                            path.display()
                        );
                    }
                    let decoded_pixels = pixels.as_chunks::<4>().0;
                    assert!(
                        decoded_pixels.iter().any(|pixel| pixel[3] == 255),
                        "{} is empty",
                        path.display()
                    );
                    assert!(
                        decoded_pixels
                            .iter()
                            .any(|pixel| pixel[3] > 0 && pixel[3] < 255),
                        "{} lacks antialiasing",
                        path.display()
                    );
                    for (pixel_index, pixel) in decoded_pixels.iter().enumerate() {
                        assert!(
                            pixel[..3].iter().all(|channel| *channel <= pixel[3]),
                            "{} has invalid premultiplied pixels",
                            path.display()
                        );
                        let (x, y) = (
                            pixel_index as u32 % FRAME_WIDTH,
                            pixel_index as u32 / FRAME_WIDTH,
                        );
                        if !(2..FRAME_WIDTH - 2).contains(&x) || !(2..FRAME_HEIGHT - 2).contains(&y)
                        {
                            assert_eq!(
                                *pixel,
                                [0; 4],
                                "{} reaches its frame border",
                                path.display()
                            );
                        }
                    }
                }
            }
            drop(factory);
            CoUninitialize();
        }
    }

    #[test]
    fn frames_wrap_without_reading_outside_the_animation() {
        assert_eq!(frame_index(0.0, 128), 0);
        assert_eq!(frame_index(0.5, 128), 64);
        assert_eq!(frame_index(1.0, 128), 0);
        assert_eq!(frame_index(-0.25, 128), 96);
        assert_eq!(frame_index(f32::NAN, 128), 0);
        assert_eq!(frame_index(f32::INFINITY, 128), 0);
        for count in [8, 16, 96, 128] {
            assert_eq!(
                frame_index(f32::from_bits(1.0_f32.to_bits() - 1), count),
                count - 1
            );
            for step in -1000..1000 {
                assert!(frame_index(step as f32 / 250.0, count) < count);
            }
        }
    }
}
