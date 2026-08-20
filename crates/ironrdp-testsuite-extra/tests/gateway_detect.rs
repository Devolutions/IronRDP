use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ironrdp_client::rdp::connect_preferring_direct;
use tokio::sync::{Notify, watch};

#[tokio::test]
async fn prefer_direct_success_skips_gateway() {
    let (_tx, mut rx) = watch::channel(false);
    let gateway_called = Arc::new(AtomicBool::new(false));
    let gateway_called_flag = Arc::clone(&gateway_called);

    let result = connect_preferring_direct(&mut rx, async { Ok::<_, &'static str>("direct") }, move || {
        gateway_called_flag.store(true, Ordering::SeqCst);
        async { Ok("gateway") }
    })
    .await;

    assert_eq!(result, Some(Ok("direct")));
    assert!(!gateway_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn prefer_direct_failure_invokes_gateway() {
    let (_tx, mut rx) = watch::channel(false);
    let gateway_called = Arc::new(AtomicBool::new(false));
    let gateway_called_flag = Arc::clone(&gateway_called);

    let result = connect_preferring_direct(&mut rx, async { Err::<&'static str, _>("direct failed") }, move || {
        gateway_called_flag.store(true, Ordering::SeqCst);
        async { Ok("gateway") }
    })
    .await;

    assert_eq!(result, Some(Ok("gateway")));
    assert!(gateway_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn prefer_direct_cancel_during_direct_skips_gateway() {
    let (tx, mut rx) = watch::channel(false);
    let gateway_called = Arc::new(AtomicBool::new(false));
    let gateway_called_flag = Arc::clone(&gateway_called);
    let direct_started = Arc::new(Notify::new());
    let direct_started_flag = Arc::clone(&direct_started);

    let connect = tokio::spawn(async move {
        connect_preferring_direct(
            &mut rx,
            async move {
                direct_started_flag.notify_one();
                core::future::pending::<Result<&'static str, &'static str>>().await
            },
            move || {
                gateway_called_flag.store(true, Ordering::SeqCst);
                async { Ok("gateway") }
            },
        )
        .await
    });

    direct_started.notified().await;
    tx.send(true).expect("close signal should send");

    assert_eq!(connect.await.expect("connect task should join"), None);
    assert!(!gateway_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn prefer_direct_cancel_during_gateway_stops_connection() {
    let (tx, mut rx) = watch::channel(false);
    let gateway_started = Arc::new(Notify::new());
    let gateway_started_flag = Arc::clone(&gateway_started);

    let connect = tokio::spawn(async move {
        connect_preferring_direct(&mut rx, async { Err::<&'static str, _>("direct failed") }, move || {
            let gateway_started_flag = Arc::clone(&gateway_started_flag);
            async move {
                gateway_started_flag.notify_one();
                core::future::pending::<Result<&'static str, &'static str>>().await
            }
        })
        .await
    });

    gateway_started.notified().await;
    tx.send(true).expect("close signal should send");

    assert_eq!(connect.await.expect("connect task should join"), None);
}
