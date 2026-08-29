//! Otip - AI-powered cross-platform video player with lookahead content moderation

mod otip_core;
mod otip_video;
mod otip_ai;
mod otip_ui;

use otip_ui::run;

fn main() -> iced::Result {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("otip=debug".parse().unwrap())
                .add_directive("iced=warn".parse().unwrap())
                .add_directive("wgpu=warn".parse().unwrap())
        )
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("Starting Otip video player");
    
    run()
}