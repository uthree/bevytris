//! Headless AI benchmark: each stage's CPU plays solo for a fixed piece
//! budget and reports attack efficiency and technique usage. Useful when
//! tuning the evaluation or the difficulty ladder.
//!
//!     cargo run -p bevytris-core --example selfplay --release [stages...]

use bevytris_core::ai::{self, AiProfile, Step};
use bevytris_core::game::Game;
use rand::SeedableRng;
use rand::rngs::StdRng;

const PIECES: u32 = 300;
const GAMES: u64 = 5;

fn run_stage(stage: u32) {
    let profile = AiProfile::for_stage(stage);
    let mut attack = 0u32;
    let mut tspins = 0u32;
    let mut tetrises = 0u32;
    let mut pieces = 0u32;
    let mut topouts = 0u32;
    for seed in 0..GAMES {
        let mut game = Game::new(seed * 7 + 1, 1);
        let mut rng = StdRng::seed_from_u64(seed);
        while !game.game_over && game.stats.pieces < PIECES {
            let queue: Vec<_> = game.queue.iter().copied().collect();
            let Some(plan) = ai::plan(
                &game.board,
                game.active,
                game.hold,
                &queue,
                0,
                game.b2b_armed(),
                game.combo(),
                &profile,
                &mut rng,
            ) else {
                game.hard_drop();
                continue;
            };
            if plan.use_hold {
                game.hold();
            }
            for step in &plan.steps {
                match step {
                    Step::Left => {
                        game.move_horizontal(-1);
                    }
                    Step::Right => {
                        game.move_horizontal(1);
                    }
                    Step::Cw => {
                        game.rotate(true);
                    }
                    Step::Ccw => {
                        game.rotate(false);
                    }
                    Step::Drop => game.sonic_drop(),
                }
            }
            game.hard_drop();
        }
        attack += game.stats.attack_sent;
        tspins += game.stats.tspins;
        tetrises += game.stats.tetrises;
        pieces += game.stats.pieces;
        if game.game_over {
            topouts += 1;
        }
    }
    println!(
        "stage {stage:2} [{:8}]  atk/100p {:5.1}   tetris {:4.1}   tspin {:4.1}   topouts {topouts}/{GAMES}",
        profile.archetype.label(),
        attack as f32 * 100.0 / pieces.max(1) as f32,
        tetrises as f32 / GAMES as f32,
        tspins as f32 / GAMES as f32,
    );
}

fn main() {
    let args: Vec<u32> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let stages = if args.is_empty() {
        vec![1, 5, 8, 13, 15, 20, 25, 28, 30]
    } else {
        args
    };
    for stage in stages {
        run_stage(stage);
    }
}
