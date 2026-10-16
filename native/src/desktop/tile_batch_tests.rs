use super::*;
use sdl2::pixels::{Color as SdlColor, PixelFormatEnum};

#[test]
#[ignore = "requires a desktop OpenGL display"]
fn rendering_atlas_matches_cpu_masks_flips_palette_splits_and_order() {
    let sdl = sdl2::init().unwrap();
    let video = sdl.video().unwrap();
    let driver = sdl2::render::drivers()
        .position(|d| d.name == "opengl")
        .unwrap() as u32;
    let window = video
        .window("Indexed atlas regression", 800, 600)
        .build()
        .unwrap();
    let mut canvas = window.into_canvas().index(driver).build().unwrap();
    for width in [320, 398, 535] {
        let gpu = GpuBatch::new(
            &video,
            &mut canvas,
            &super::super::benchmark::fixture(width, 0),
        )
        .unwrap();
        for tick in [0, 23] {
            let batch = super::super::benchmark::fixture(width, tick);
            let mut frame = Frame::new(width);
            batch.cpu_draw(&mut frame);
            let rect = Rect::new(11, 13, width as u32, HEIGHT as u32);
            canvas.set_draw_color(SdlColor::RGB(17, 23, 31));
            canvas.clear();
            gpu.upload_palettes(&mut canvas, &batch).unwrap();
            gpu.draw(&mut canvas, rect, &batch).unwrap();
            let actual = canvas.read_pixels(rect, PixelFormatEnum::RGB24).unwrap();
            assert_eq!(actual.as_slice(), frame.rgb(), "width={width}, tick={tick}");
            // SDL textured rendering still works after the prototype restores GL state.
            let creator = canvas.texture_creator();
            let mut texture = creator
                .create_texture_streaming(PixelFormatEnum::RGB24, 1, 1)
                .unwrap();
            texture.update(None, &[255, 0, 255], 3).unwrap();
            canvas.copy(&texture, None, Rect::new(0, 0, 4, 4)).unwrap();
            assert_eq!(
                canvas
                    .read_pixels(Rect::new(0, 0, 1, 1), PixelFormatEnum::RGB24)
                    .unwrap(),
                [255, 0, 255]
            );
            canvas.present();
        }
    }
}
