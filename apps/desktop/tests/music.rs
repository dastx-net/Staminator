use staminator_desktop::music::MusicService;
use std::{
    fs, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

fn wait_for_error(player: &MusicService) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(error) = player.try_error() {
            return error;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("music worker did not report an error");
}

#[test]
fn service_recovers_after_directory_is_created_and_refreshed() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "staminator-music-integration-{}-{nonce}",
        std::process::id()
    ));
    let player = MusicService::new(&root, true, 0.25);
    assert!(wait_for_error(&player).contains("cannot scan music directory"));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("broken.mp3"), b"not a valid audio file").unwrap();
    player.refresh().unwrap();
    assert!(wait_for_error(&player).contains("broken.mp3"));
    player.stop().unwrap();
    drop(player);
    fs::remove_dir_all(root).unwrap();
}
