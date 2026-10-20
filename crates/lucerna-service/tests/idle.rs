use std::time::Duration;

use lucerna_service::idle::{WorkTracker, wait_idle};
use tokio::sync::watch;
use tokio::time::Instant;

#[tokio::test(start_paused = true)]
async fn idle_after_timeout_with_nobody() {
    let (_c, cr) = watch::channel(0usize);
    let w = WorkTracker::new();
    let t0 = Instant::now();
    wait_idle(cr, w.watch(), Duration::from_secs(60)).await;
    assert_eq!(t0.elapsed(), Duration::from_secs(60));
}

#[tokio::test(start_paused = true)]
async fn a_client_resets_the_clock() {
    let (c, cr) = watch::channel(1usize);
    let w = WorkTracker::new();
    let t0 = Instant::now();
    let driver = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        c.send(0).unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        c.send(1).unwrap();
        tokio::time::sleep(Duration::from_secs(10)).await;
        c.send(0).unwrap();
        c // mantém o emissor vivo até o fim
    });
    wait_idle(cr, w.watch(), Duration::from_secs(60)).await;
    assert_eq!(t0.elapsed(), Duration::from_secs(110));
    drop(driver.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn pending_work_holds_the_service() {
    let (_c, cr) = watch::channel(0usize);
    let w = WorkTracker::new();
    let guard = w.hold();
    assert_eq!(w.pending(), 1);
    let t0 = Instant::now();
    let h = tokio::spawn(wait_idle(cr, w.watch(), Duration::from_secs(60)));
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert!(!h.is_finished(), "não sai com trabalho pendente");
    drop(guard);
    assert_eq!(w.pending(), 0);
    h.await.unwrap();
    assert_eq!(t0.elapsed(), Duration::from_secs(660));
}

#[tokio::test(start_paused = true)]
async fn closed_senders_do_not_spin() {
    // Emissor largado com conexões em 0: o valor é o último; sai no tempo certo.
    let (c, cr) = watch::channel(0usize);
    drop(c);
    let w = WorkTracker::new();
    let t0 = Instant::now();
    wait_idle(cr, w.watch(), Duration::from_secs(60)).await;
    assert_eq!(t0.elapsed(), Duration::from_secs(60));
}
