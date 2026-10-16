use super::*;

#[test]
fn filtered_wave_pixels_preserve_transparency_priority_cram_and_fades() {
    let mut frame = Frame::default();
    frame.clear([0, 0, 255]);
    let mut palette = [[0; 3]; 16];
    palette[1] = [255, 0, 0];
    let slot = frame.slot(PaletteRef::Cram(0, &palette));
    let taps = [Some((slot, 1)), None, None, None];
    let weights = [0.5, 0.5, 0.0, 0.0];
    frame.filtered_pixel(0, 30, taps, weights);
    assert_eq!(pixel(&frame, 0, 30), [128, 0, 128]);
    palette[1] = [0, 255, 0];
    frame.change_cram_line(CramChange {
        line: 30,
        cram_line: 0,
        colors: palette,
    });
    assert_eq!(pixel(&frame, 0, 30), [0, 128, 128]);
    frame.fade(Fade::Out(FADE_STEPS));
    assert_eq!(pixel(&frame, 0, 30), [0, 128, 0]);
    frame.mark_high_sprites(true);
    frame.pixel(1, 30, [255; 3]);
    frame.mark_high_sprites(false);
    frame.behind_high_sprites(true);
    frame.filtered_pixel(1, 30, taps, weights);
    assert_eq!(pixel(&frame, 1, 30), [255; 3]);
    frame.clear([0; 3]);
    assert_eq!(pixel(&frame, 0, 30), [0; 3]);
}

#[test]
fn widescreen_pixels_tiles_and_raster_changes_cover_both_sides() {
    let mut frame = Frame::new(crate::WIDE_WIDTH);
    let mut palette = [[0; 3]; 16];
    palette[1] = [255, 0, 0];
    let tile = Tile([0x11; 32]);
    let left = frame.view_left();
    let right = frame.view_right();
    frame.clear_backdrop(PaletteRef::Cram(0, &palette), 1);
    frame.tile(
        &tile,
        PaletteRef::Cram(0, &palette),
        left - 4,
        30,
        Flip::default(),
    );
    frame.tile(
        &tile,
        PaletteRef::Cram(0, &palette),
        right - 4,
        30,
        Flip::default(),
    );
    palette[1] = [0, 255, 0];
    frame.change_cram_line(CramChange {
        line: 32,
        cram_line: 0,
        colors: palette,
    });
    frame.clip_lines(Some(34));
    frame.pixel(left, 33, [0, 0, 255]);
    frame.pixel(right - 1, 34, [0, 0, 255]);
    let pixel = |x, y| {
        let at = (y * frame.width() + x) * 3;
        &frame.rgb()[at..at + 3]
    };
    for x in [0, frame.width() - 1] {
        assert_eq!(pixel(x, 31), [255, 0, 0]);
        assert_eq!(pixel(x, 32), [0, 255, 0]);
        assert_eq!(pixel(x, 34), [0, 255, 0]);
    }
    assert_eq!(pixel(0, 33), [0, 0, 255]);
    let mut ppm = Vec::new();
    frame.write_ppm(&mut ppm).unwrap();
    assert!(ppm.starts_with(b"P6\n398 224\n255\n"));
    assert_eq!(frame.rgb().len(), crate::WIDE_WIDTH * HEIGHT * 3);
}

#[test]
fn widescreen_view_stays_inside_layout_and_centres_ui() {
    let mut frame = Frame::new(crate::WIDE_WIDTH);
    frame.align_world(0, 4096);
    assert_eq!((frame.view_left(), frame.view_right()), (0, 398));
    frame.align_world(1024, 4096);
    assert_eq!((frame.view_left(), frame.view_right()), (-39, 359));
    frame.align_world(3776, 4096);
    assert_eq!((frame.view_left(), frame.view_right()), (-78, 320));
    assert_eq!(frame.world_camera_delta(), -39);
    frame.align_world(0, 320);
    assert_eq!((frame.view_left(), frame.view_right()), (-39, 359));
    frame.align_world(16, 256);
    assert_eq!((frame.view_left(), frame.view_right()), (-87, 311));
    frame.center_content();
    assert_eq!((frame.view_left(), frame.view_right()), (-39, 359));
    let mut original = Frame::default();
    original.align_world(0, 4096);
    assert_eq!((original.view_left(), original.view_right()), (0, 320));
}

fn pixel(frame: &Frame, x: usize, y: usize) -> &[u8] {
    let start = (y * WIDTH + x) * 3;
    &frame.rgb()[start..start + 3]
}

#[test]
fn packed_tile_transparency_flip_and_clipping() {
    let mut tile = Tile([0; 32]);
    tile.0[0] = 0x12;
    let mut palette = [[0; 3]; 16];
    palette[1] = [255, 0, 0];
    palette[2] = [0, 255, 0];
    let mut frame = Frame::default();
    frame.clear([1, 2, 3]);
    frame.tile(&tile, &palette, -1, 0, Flip::default());
    assert_eq!(pixel(&frame, 0, 0), palette[2]);
    assert_eq!(pixel(&frame, 1, 0), [1, 2, 3]);
    frame.tile(&tile, &palette, 0, 0, Flip { x: true, y: true });
    assert_eq!(pixel(&frame, 7, 7), palette[1]);
    assert_eq!(pixel(&frame, 6, 7), palette[2]);
    // The bottom-right corner clips the rest of the tile.
    frame.tile(
        &tile,
        &palette,
        WIDTH as i32 - 1,
        HEIGHT as i32 - 1,
        Flip::default(),
    );
    assert_eq!(pixel(&frame, WIDTH - 1, HEIGHT - 1), palette[1]);
    // Fully off-screen tiles are safe and do not wrap to another row.
    let before = frame.checksum();
    frame.tile(
        &tile,
        &palette,
        WIDTH as i32,
        HEIGHT as i32,
        Flip::default(),
    );
    assert_eq!(frame.checksum(), before);
}

#[test]
fn high_plane_passes_cover_low_sprites_but_not_high_ones() {
    let mut tile = Tile([0; 32]);
    tile.0.fill(0x11);
    let mut red = [[0; 3]; 16];
    red[1] = [255, 0, 0];
    let mut blue = red;
    blue[1] = [0, 0, 255];
    let mut frame = Frame::default();
    frame.clear([1, 2, 3]);
    frame.tile(&tile, &red, 0, 0, Flip::default()); // a low sprite
    frame.mark_high_sprites(true);
    frame.tile(&tile, &red, 8, 0, Flip::default()); // a high sprite
    frame.mark_high_sprites(false);
    frame.behind_high_sprites(true);
    for x in [0, 8] {
        frame.tile(&tile, &blue, x, 0, Flip::default()); // high plane cells
    }
    frame.behind_high_sprites(false);
    assert_eq!(pixel(&frame, 0, 0), [0, 0, 255]);
    assert_eq!(pixel(&frame, 8, 0), [255, 0, 0]);
    // A low sprite in front of a high one takes its pixels behind the
    // plane cells too.
    frame.mark_high_sprites(true);
    frame.tile(&tile, &red, 16, 0, Flip::default());
    frame.mark_high_sprites(false);
    frame.tile(&tile, &red, 16, 0, Flip::default());
    frame.behind_high_sprites(true);
    frame.tile(&tile, &blue, 16, 0, Flip::default());
    frame.behind_high_sprites(false);
    assert_eq!(pixel(&frame, 16, 0), [0, 0, 255]);
    // A new frame forgets the sprites.
    frame.clear([1, 2, 3]);
    frame.behind_high_sprites(true);
    frame.tile(&tile, &blue, 8, 0, Flip::default());
    assert_eq!(pixel(&frame, 8, 0), [0, 0, 255]);
}

#[test]
fn clipped_lines_drop_writes_until_reset() {
    let mut tile = Tile([0; 32]);
    tile.0.fill(0x11);
    let mut red = [[0; 3]; 16];
    red[1] = [255, 0, 0];
    let mut frame = Frame::default();
    frame.clear([1, 2, 3]);
    frame.clip_lines(Some(201));
    frame.tile(&tile, &red, 0, 196, Flip::default());
    frame.pixel(20, 201, [9, 9, 9]);
    assert_eq!(pixel(&frame, 0, 200), [255, 0, 0]);
    assert_eq!(pixel(&frame, 0, 201), [1, 2, 3]);
    assert_eq!(pixel(&frame, 20, 201), [1, 2, 3]);
    frame.clip_lines(None);
    frame.pixel(20, 201, [9, 9, 9]);
    assert_eq!(pixel(&frame, 20, 201), [9, 9, 9]);
}

#[test]
fn a_palette_split_recolours_its_line_below_the_split_only() {
    let mut tile = Tile([0; 32]);
    tile.0.fill(0x11);
    let mut normal = [[0; 3]; 16];
    normal[1] = [255, 0, 0];
    let mut other = normal;
    other[1] = [0, 255, 0];
    let mut reloaded = normal;
    reloaded[1] = [0, 0, 255];
    let mut frame = Frame::default();
    frame.clear([1, 2, 3]);
    frame.tile(&tile, PaletteRef::Cram(1, &normal), 0, 196, Flip::default());
    // The change reaches pixels drawn before and after it.
    frame.change_cram_line(CramChange {
        line: 200,
        cram_line: 1,
        colors: reloaded,
    });
    frame.tile(&tile, PaletteRef::Cram(2, &other), 8, 196, Flip::default());
    frame.tile(
        &tile,
        PaletteRef::Cram(1, &normal),
        16,
        196,
        Flip::default(),
    );
    assert_eq!(pixel(&frame, 0, 199), [255, 0, 0]);
    assert_eq!(pixel(&frame, 0, 200), [0, 0, 255]);
    assert_eq!(pixel(&frame, 16, 199), [255, 0, 0]);
    assert_eq!(pixel(&frame, 16, 203), [0, 0, 255]);
    assert_eq!(pixel(&frame, 8, 200), [0, 255, 0]);
    // The next frame starts with the line as the CRAM copy has it.
    frame.clear([1, 2, 3]);
    frame.tile(
        &tile,
        PaletteRef::Cram(1, &normal),
        16,
        196,
        Flip::default(),
    );
    assert_eq!(pixel(&frame, 16, 203), [255, 0, 0]);
}

/// The VDP looks each pixel's CRAM index up, not its colour: another
/// line and a native copy holding exactly the changed line's colours
/// keep them below the change.
#[test]
fn a_cram_change_leaves_lines_and_copies_that_share_its_colours() {
    let mut tile = Tile([0; 32]);
    tile.0.fill(0x12);
    let mut normal = [[0; 3]; 16];
    normal[1] = [255, 0, 0];
    normal[2] = [36, 72, 109];
    let mut reloaded = normal;
    reloaded[1] = [0, 0, 255];
    reloaded[2] = [0, 0, 0];
    let mut frame = Frame::default();
    frame.clear([1, 2, 3]);
    frame.change_cram_line(CramChange {
        line: 200,
        cram_line: 1,
        colors: reloaded,
    });
    let lines = [normal; 4];
    let cram = cram_lines(&lines);
    frame.tile(&tile, cram[1], 0, 196, Flip::default());
    frame.tile(&tile, cram[2], 8, 196, Flip::default());
    frame.tile(&tile, &normal, 16, 196, Flip::default());
    frame.pixel(24, 200, normal[1]);
    frame.rect(25, 200, 1, 1, normal[2]);
    assert_eq!(pixel(&frame, 0, 199), [255, 0, 0]);
    assert_eq!(pixel(&frame, 0, 200), [0, 0, 255]);
    assert_eq!(pixel(&frame, 1, 200), [0, 0, 0]);
    for x in [8, 16] {
        assert_eq!(pixel(&frame, x, 200), [255, 0, 0]);
        assert_eq!(pixel(&frame, x + 1, 203), [36, 72, 109]);
    }
    assert_eq!(pixel(&frame, 24, 200), [255, 0, 0]);
    assert_eq!(pixel(&frame, 25, 200), [36, 72, 109]);
}

/// A fade is CRAM contents: a CRAM change replaces the faded colours of
/// its line from its display line down, added before or after the
/// fade, and leaves other lines, native copies and direct colours
/// faded.
#[test]
fn a_cram_change_replaces_the_faded_colours_of_its_line() {
    let mut tile = Tile([0; 32]);
    tile.0.fill(0x11);
    let mut normal = [[0; 3]; 16];
    normal[1] = [255, 255, 255];
    let mut reloaded = normal;
    reloaded[1] = [0, 0, 255];
    let change = CramChange {
        line: 200,
        cram_line: 1,
        colors: reloaded,
    };
    for before in [false, true] {
        let mut frame = Frame::default();
        frame.clear([0; 3]);
        if before {
            frame.change_cram_line(change);
        }
        frame.tile(&tile, PaletteRef::Cram(1, &normal), 0, 196, Flip::default());
        frame.tile(&tile, PaletteRef::Cram(2, &normal), 8, 196, Flip::default());
        frame.tile(&tile, &normal, 16, 196, Flip::default());
        frame.pixel(24, 200, [255, 255, 255]);
        frame.fade(Fade::Out(FADE_STEPS));
        if !before {
            frame.change_cram_line(change);
        }
        // Drawn after the fade: not faded, and changed below line 200.
        frame.tile(
            &tile,
            PaletteRef::Cram(1, &normal),
            32,
            196,
            Flip::default(),
        );
        assert_eq!(pixel(&frame, 0, 199), [0, 0, 0]);
        assert_eq!(
            pixel(&frame, 0, 200),
            [0, 0, 255],
            "change before: {before}"
        );
        for x in [8, 16, 24] {
            assert_eq!(pixel(&frame, x, 200), [0, 0, 0]);
        }
        assert_eq!(pixel(&frame, 32, 199), [255, 255, 255]);
        assert_eq!(pixel(&frame, 32, 200), [0, 0, 255]);
    }
    // Part-way through a fade the faded line keeps its CRAM line.
    let mut frame = Frame::default();
    frame.clear([0; 3]);
    frame.tile(&tile, PaletteRef::Cram(1, &normal), 0, 196, Flip::default());
    frame.fade(Fade::Out(4));
    frame.change_cram_line(change);
    let dimmed = Fade::Out(4).channel(255);
    assert_eq!(pixel(&frame, 0, 199), [dimmed; 3]);
    assert_eq!(pixel(&frame, 0, 200), [0, 0, 255]);
}

/// The backdrop is the CRAM entry register $07 selects: it follows CRAM
/// changes and fades like any pixel of that line.
#[test]
fn the_backdrop_is_a_cram_entry() {
    let mut lines = [[[0; 3]; 16]; 4];
    lines[0][0] = [255, 0, 0];
    lines[2][0] = [0, 255, 0];
    let cram = cram_lines(&lines);
    let mut frame = Frame::default();
    frame.clear_backdrop(cram[0], 0);
    let mut changed = lines[0];
    changed[0] = [0, 0, 255];
    frame.change_cram_line(CramChange {
        line: 100,
        cram_line: 0,
        colors: changed,
    });
    assert_eq!(pixel(&frame, 5, 99), [255, 0, 0]);
    assert_eq!(pixel(&frame, 5, 100), [0, 0, 255]);
    frame.fade(Fade::Out(FADE_STEPS));
    assert_eq!(pixel(&frame, 5, 99), [0, 0, 0]);
    assert_eq!(pixel(&frame, 5, 100), [0, 0, 255]);
    // Register $07 = $20: line 2, colour 0.
    let mut frame = Frame::default();
    frame.clear_backdrop(cram[2], 0);
    assert_eq!(pixel(&frame, 319, 223), [0, 255, 0]);
    // A new frame drops the backdrop's palette with the rest.
    frame.clear([1, 2, 3]);
    assert_eq!(pixel(&frame, 0, 0), [1, 2, 3]);
}

/// A fade keeps the colours the frame shows when it runs; what is drawn
/// later is not faded. Many distinct palettes in one frame still draw.
#[test]
fn fades_reach_what_is_drawn_so_far_and_slots_overflow_safely() {
    let mut tile = Tile([0; 32]);
    tile.0.fill(0x11);
    let mut white = [[0; 3]; 16];
    white[1] = [255, 255, 255];
    let mut frame = Frame::default();
    frame.clear([0; 3]);
    frame.tile(&tile, PaletteRef::Cram(0, &white), 0, 0, Flip::default());
    frame.fade(Fade::Out(FADE_STEPS));
    frame.tile(&tile, PaletteRef::Cram(0, &white), 8, 0, Flip::default());
    assert_eq!(pixel(&frame, 0, 0), [0, 0, 0]);
    assert_eq!(pixel(&frame, 8, 0), [255, 255, 255]);
    for n in 0..SLOT_LIMIT + 8 {
        let mut palette = [[0; 3]; 16];
        palette[1] = [n as u8, (n >> 8) as u8, 7];
        frame.tile(
            &tile,
            &palette,
            (n % 40) as i32 * 8,
            8 + (n / 40) as i32 * 8,
            Flip::default(),
        );
    }
    for n in [0, SLOT_LIMIT - 1, SLOT_LIMIT, SLOT_LIMIT + 7] {
        let (x, y) = (n % 40 * 8, 8 + n / 40 * 8);
        assert_eq!(pixel(&frame, x, y), [n as u8, (n >> 8) as u8, 7]);
    }
    assert_eq!(pixel(&frame, 8, 0), [255, 255, 255]);
}

#[test]
fn fades_step_cram_nibbles_like_the_cartridge() {
    let white = md_color(0x0eee);
    assert_eq!(white, [255, 255, 255]);
    let grey = md_color(0x0444)[0]; // level 2
    // $67A: one nibble decrement per update; level 2 is black after 4.
    // CRAM keeps only bits 1-3, so odd shadow nibbles show the level
    // below: white is level 1 after 12 and black from 13.
    assert_eq!(Fade::Out(0).channel(grey), grey);
    assert_eq!(Fade::Out(1).channel(grey), md_color(0x0222)[0]);
    assert_eq!(Fade::Out(4).channel(grey), 0);
    assert_eq!(Fade::Out(12).channel(255), md_color(0x0222)[0]);
    assert_eq!(Fade::Out(13).channel(255), 0);
    assert_eq!(Fade::Out(FADE_STEPS).channel(255), 0);
    // $5EA: increments toward the colour; darker channels arrive first.
    assert_eq!(Fade::In(0).channel(255), 0);
    assert_eq!(Fade::In(4).channel(grey), grey);
    assert_eq!(Fade::In(4).channel(255), md_color(0x0444)[0]);
    assert_eq!(Fade::In(FADE_STEPS).channel(255), 255);
    let mut fade = Fade::Out(0);
    for _ in 0..20 {
        fade = fade.advance();
    }
    assert_eq!(fade, Fade::Out(FADE_STEPS));
    assert!(fade.finished() && !Fade::In(3).finished());
    let mut frame = Frame::default();
    frame.clear([255, 255, 255]);
    let before = frame.checksum();
    frame.fade(Fade::In(FADE_STEPS));
    assert_eq!(frame.checksum(), before);
    frame.fade(Fade::Out(FADE_STEPS));
    assert!(frame.rgb().iter().all(|&c| c == 0));
}
