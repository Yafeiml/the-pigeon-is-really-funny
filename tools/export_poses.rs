#![allow(dead_code)]

mod motion {
    include!("../src/motion.rs");

    pub fn export() -> String {
        let mut output = String::from("{");
        for (index, (name, action, seconds)) in [
            ("walk", Mode::Walk, STRIDE / WALK_SPEED),
            ("look", Mode::Look, 3.4),
            ("peck", Mode::Peck, PECK_SECONDS),
        ].into_iter().enumerate() {
            if index > 0 { output.push(','); }
            output.push_str(&format!("\"{name}\":{{\"seconds\":{seconds},\"poses\":["));
            let frames = sprite_frame_count(name, cfg!(feature = "painted"));
            for frame in 0..frames {
                let phase = frame as f32 / frames as f32;
                let mut model = Motion::new();
                model.action = action;
                model.phase = phase;
                model.action_time = phase * seconds;
                model.elapsed = model.action_time;
                model.walk_weight = if action == Mode::Walk { 1.0 } else { 0.0 };
                model.transition_time = 0.28;
                let p = model.pose();
                if frame > 0 { output.push(','); }
                output.push_str(&format!(
                    "{{\"head_x\":{},\"head_y\":{},\"body_y\":{},\"body_pitch\":{},\"near_foot_x\":{},\"near_foot_y\":{},\"far_foot_x\":{},\"far_foot_y\":{},\"look\":{},\"peck\":{},\"facing\":{}",
                    p.head_x,p.head_y,p.body_y,p.body_pitch,p.near_foot_x,p.near_foot_y,
                    p.far_foot_x,p.far_foot_y,p.look,p.peck,p.facing
                ));
                #[cfg(feature = "painted")]
                if action == Mode::Peck {
                    output.push_str(&format!(",\"time_seconds\":{}", model.action_time));
                }
                output.push('}');
            }
            output.push_str("]}");
        }
        output.push('}');
        output
    }
}

fn main() -> std::io::Result<()> {
    let destination = std::env::args().nth(1).expect("output JSON path required");
    std::fs::write(destination, motion::export())
}
