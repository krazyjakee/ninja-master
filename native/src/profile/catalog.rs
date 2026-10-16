//! Local equivalents of the public game-64 core set, inventoried 2026-10-16.
//! Names identify the reference challenges; descriptions and native predicates
//! are maintained here. No RA badge artwork, credentials or submissions.
#[derive(Clone, Copy)]
pub struct Achievement {
    pub name: &'static str,
    pub description: &'static str,
    pub points: u16,
}
macro_rules! a {
    ($name:literal, $points:literal, $description:literal) => {
        Achievement {
            name: $name,
            points: $points,
            description: $description,
        }
    };
}
pub const CATALOG: [Achievement; 49] = [
    a!("Zeed's Resurrection", 5, "Clear round 1."),
    a!("Secret Entry", 5, "Clear round 2."),
    a!("Body Weapon", 5, "Clear round 3."),
    a!("Destruction", 5, "Clear round 4."),
    a!("Electric Demon", 10, "Clear round 5."),
    a!("Traps", 10, "Clear round 6."),
    a!("The Final Confrontation", 25, "Clear the final round."),
    a!(
        "The True Ninja Master",
        50,
        "Clear all seven rounds on Expert without continuing."
    ),
    a!(
        "Acrophobia",
        10,
        "Round 1 midboss: no jumps, damage, stars or magic."
    ),
    a!("Spear Terror", 10, "Round 2 midboss: no damage or magic."),
    a!(
        "The Floor Is... Acid?",
        10,
        "Round 3 midboss: stay off the floor; no damage or magic."
    ),
    a!(
        "Surfing Is a Martial Art",
        10,
        "Round 4 midboss: board attacks only; magic except Mijin is allowed."
    ),
    a!(
        "Iron Frame, Crimson Flames",
        10,
        "Round 5 midboss: death kicks only; no damage or magic."
    ),
    a!(
        "Feathered Freak",
        10,
        "Round 6 midboss: no damage or magic."
    ),
    a!(
        "Clash in the Caverns",
        10,
        "Round 1 boss: no damage, stars or magic."
    ),
    a!("Brain Surgery", 25, "Round 2 boss: no damage or magic."),
    a!(
        "Great and Grotesque",
        25,
        "Round 3 boss: no damage or magic."
    ),
    a!("Hydro-Plane", 25, "Round 4 boss: no damage or magic."),
    a!("Mecha-Gone-Zilla", 25, "Round 5 boss: no damage or magic."),
    a!(
        "Ex-Zeed-ing Expectations",
        25,
        "Round 6 boss: no damage or magic."
    ),
    a!(
        "At Death's Dimension",
        50,
        "Final boss: no damage or magic."
    ),
    a!(
        "Shuriken Collector",
        2,
        "Carry 100 stars above your starting stock."
    ),
    a!(
        "Shuriken Prepper",
        5,
        "Carry 200 stars above your starting stock."
    ),
    a!(
        "Shuriken Hoarder",
        10,
        "Carry 300 stars above your starting stock."
    ),
    a!("Nix on Ninjutsu I", 5, "Clear round 1 without magic."),
    a!("Nix on Ninjutsu II", 10, "Clear round 2 without magic."),
    a!("Nix on Ninjutsu III", 10, "Clear round 3 without magic."),
    a!("Nix on Ninjutsu IV", 10, "Clear round 4 without magic."),
    a!("Nix on Ninjutsu V", 10, "Clear round 5 without magic."),
    a!("Nix on Ninjutsu VI", 10, "Clear round 6 without magic."),
    a!("Nix on Ninjutsu VII", 25, "Clear round 7 without magic."),
    a!("Ninjits-who?", 25, "Clear all seven rounds without magic."),
    a!(
        "Technically Terrific",
        10,
        "Clear a round at maximum health."
    ),
    a!(
        "Hold Your Fire",
        10,
        "Start a round with 10 or more stars; clear it without throwing."
    ),
    a!(
        "Perfect Assassin",
        25,
        "Clear a round without damage, stars or magic."
    ),
    a!(
        "Art of Self-Preservation",
        10,
        "Build your spare lives up to nine on Expert."
    ),
    a!(
        "Genin's Marks",
        3,
        "Reach 50,000 points on Normal, Hard or Expert."
    ),
    a!(
        "Chunin's Marks",
        5,
        "Reach 150,000 points on Normal, Hard or Expert."
    ),
    a!(
        "Jonin's Marks",
        10,
        "Reach 300,000 points on Normal, Hard or Expert."
    ),
    a!(
        "Shinobi Secrets I",
        3,
        "Explore the first hidden area in round 1."
    ),
    a!(
        "Shinobi Secrets II",
        3,
        "Explore the second hidden area in round 1."
    ),
    a!(
        "Shinobi Secrets III",
        4,
        "Explore the hidden area in round 2."
    ),
    a!(
        "Shinobi Secrets IV",
        3,
        "Explore the hidden area in round 3."
    ),
    a!(
        "Shinobi Secrets V",
        5,
        "Collect the extra life in round 4's hidden area."
    ),
    a!(
        "Shinobi Secrets VI",
        3,
        "Explore the first hidden area in round 5."
    ),
    a!(
        "Shinobi Secrets VII",
        3,
        "Explore the second hidden area in round 5."
    ),
    a!(
        "Shinobi Secrets VIII",
        4,
        "Explore the third hidden area in round 5."
    ),
    a!(
        "Shinobi Secrets IX",
        4,
        "Explore the fourth hidden area in round 5."
    ),
    a!(
        "Shinobi Secrets X",
        3,
        "Explore the hidden area in round 7."
    ),
];
