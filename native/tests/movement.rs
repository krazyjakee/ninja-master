use ninja_master_native::{
    assets::{PlayerAssets, StageAssets},
    input::{Button, Input},
    movement::{Player, state},
    rom::regions::prepare,
    stage::StageId,
};

#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn round_2_2_double_jump_under_overhang_does_not_climb_off_map() {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    for region in ["us", "eu", "jp"] {
        let rom = prepare(std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap())
            .unwrap();
        PlayerAssets::load(&rom).unwrap();
        let terrain = StageAssets::load_stage(&rom, StageId::new(2, 2, 0)).unwrap();
        let mut player = Player::new(2145, 480);
        player.x += 192;
        player.tick(Input::default(), &terrain);
        let right = Button::Right.mask();
        let jump = right | Button::Jump.mask();
        let mut previous = 0;
        let mut somersault = false;
        let mut minimum_y = 480;
        // The run and two jump presses from the reported save, starting
        // before the approach to the overhang at x=2360.
        for (ticks, held) in [
            (4, right),
            (5, 0),
            (22, right),
            (27, jump),
            (11, right),
            (32, jump),
            (100, right),
        ] {
            for _ in 0..ticks {
                player.tick(
                    Input {
                        held,
                        pressed: held & !previous,
                    },
                    &terrain,
                );
                previous = held;
                somersault |= player.animator.state == state::SOMERSAULT;
                minimum_y = minimum_y.min(player.position().1);
                assert!(
                    player.position().1 >= 300,
                    "{region}: climbed the wall at {:?}",
                    player.position()
                );
            }
        }
        assert!(somersault, "{region}: did not double jump");
        assert!(minimum_y < 320, "{region}: did not reach the overhang");
        assert!(
            player.position().1 > minimum_y,
            "{region}: did not fall back down"
        );
        assert!(player.grounded, "{region}: did not land");
    }
}
