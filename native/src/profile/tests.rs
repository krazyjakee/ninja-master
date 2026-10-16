use super::*;
fn path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ninja-profile-{}-{label}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[test]
fn complete_reference_inventory_has_49_unique_challenges_and_590_points() {
    assert_eq!(CATALOG.len(), 49);
    assert_eq!(CATALOG.iter().map(|a| a.points).sum::<u16>(), 590);
    assert_eq!(
        CATALOG
            .iter()
            .map(|a| a.name)
            .collect::<BTreeSet<_>>()
            .len(),
        49
    );
}
#[test]
fn rewinds_retries_and_restart_do_not_duplicate_credit_or_notifications() {
    let path = path("rewind");
    let mut p = Profile::load(Some(path.clone()));
    let mut run = Run::default();
    p.begin(&mut run);
    run.stats.ticks = 600;
    run.stats.shurikens = 8;
    run.stats.enemies = 2;
    run.award(0);
    run.best_rounds[0] = Some(600);
    p.observe(&run);
    p.observe(&run);
    assert_eq!(p.data.lifetime, run.stats);
    assert_eq!(p.notices.len(), 1);
    let stamp = p.data.unlocked[&0];
    p.flush();
    drop(p);
    let mut p = Profile::load(Some(path.clone()));
    let mut rewind = run.clone();
    rewind.stats.ticks = 60;
    rewind.stats.shurikens = 1;
    rewind.stats.enemies = 0;
    p.observe(&rewind);
    p.observe(&run);
    assert_eq!(p.data.lifetime, run.stats);
    assert!(p.notices.is_empty());
    assert_eq!(p.data.unlocked[&0], stamp);
    run.stats.shurikens = 10;
    p.observe(&run);
    assert_eq!(p.data.lifetime.shurikens, 10);
    let mut next = Run::default();
    p.begin(&mut next);
    assert_ne!(run.id, next.id);
    drop(p);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn practice_is_sticky_and_keeps_separate_totals_without_unlocking_loaded_candidates() {
    let mut p = Profile::default();
    let mut run = Run::default();
    p.begin(&mut run);
    run.stats.ticks = 60;
    p.observe(&run);
    let clean = run.clone();
    run.award(0);
    p.taint(&mut run);
    assert!(p.data.unlocked.is_empty());
    run.stats.ticks = 120;
    p.observe(&run);
    assert_eq!(p.data.lifetime.ticks, 60);
    assert_eq!(p.data.practice.ticks, 60);
    let mut rewind = clean;
    rewind.award(1);
    rewind.stats.ticks = 180;
    p.observe(&rewind);
    assert!(p.data.unlocked.is_empty());
    assert_eq!(p.data.practice.ticks, 120);
    assert!(p.data.last_run.as_ref().unwrap().practice);
}
#[test]
fn missing_fields_migrate_and_corrupt_or_future_profiles_are_preserved() {
    let path = path("migration");
    std::fs::write(&path, br#"{"lifetime":{"deaths":3}}"#).unwrap();
    let mut p = Profile::load(Some(path.clone()));
    assert_eq!(p.data.lifetime.deaths, 3);
    let mut run = Run::default();
    p.begin(&mut run);
    p.flush();
    drop(p);
    for bytes in [b"invalid".as_slice(), br#"{"version":99}"#.as_slice()] {
        std::fs::write(&path, bytes).unwrap();
        let mut p = Profile::load(Some(path.clone()));
        assert!(!p.status.is_empty());
        let mut run = Run::default();
        p.begin(&mut run);
        p.flush();
        drop(p);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::remove_file(path).unwrap();
}
#[test]
fn notification_queue_displays_every_unlock_for_four_seconds() {
    let mut p = Profile::default();
    let mut run = Run::default();
    p.begin(&mut run);
    run.award(0);
    run.award(1);
    p.observe(&run);
    p.tick();
    assert_eq!(p.notices.front(), Some(&0));
    for _ in 0..600 {
        p.tick_without_notices();
    }
    assert_eq!(p.notices.front(), Some(&0));
    assert_eq!(
        p.notice_ticks, 240,
        "profile browsing pauses the popup timer"
    );
    assert_eq!(p.last_flush, 1, "persistence still advances while browsing");
    for _ in 0..239 {
        p.tick();
    }
    assert_eq!(p.notices.front(), Some(&0));
    p.tick();
    assert_eq!(p.notices.front(), Some(&1));
    for _ in 0..240 {
        p.tick();
    }
    assert!(p.notices.is_empty());
}
#[test]
fn formatted_playtime_does_not_wrap_at_24_hours() {
    assert_eq!(duration(60 * 60 * 60 * 25 + 60 * 61), "25:01:01");
}
