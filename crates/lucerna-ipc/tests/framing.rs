use lucerna_ipc::framing::*;
use tokio::io::AsyncWriteExt;

async fn reader_over(chunks: Vec<Vec<u8>>, max: usize) -> LineReader<tokio::io::DuplexStream> {
    let (mut a, b) = tokio::io::duplex(64);
    tokio::spawn(async move {
        for c in chunks {
            a.write_all(&c).await.unwrap();
            tokio::task::yield_now().await;
        }
    });
    LineReader::new(b, max)
}

#[tokio::test]
async fn joins_split_lines_and_splits_joined_ones() {
    let mut r = reader_over(
        vec![
            b"{\"a\":".to_vec(),
            b"1}\n{\"b\":2}\n\n{\"c\"".to_vec(),
            b":3}\r\n".to_vec(),
        ],
        1024,
    )
    .await;
    assert_eq!(r.next_line().await.unwrap().unwrap(), r#"{"a":1}"#);
    assert_eq!(r.next_line().await.unwrap().unwrap(), r#"{"b":2}"#);
    assert_eq!(r.next_line().await.unwrap().unwrap(), r#"{"c":3}"#);
    assert!(r.next_line().await.unwrap().is_none());
}

#[tokio::test]
async fn utf8_split_across_reads() {
    let s = "{\"m\":\"não perturbe ☕\"}\n".as_bytes().to_vec();
    let cut = s.iter().position(|&b| b >= 0x80).unwrap() + 1; // no meio de "ã"
    let mut r = reader_over(vec![s[..cut].to_vec(), s[cut..].to_vec()], 1024).await;
    assert_eq!(
        r.next_line().await.unwrap().unwrap(),
        "{\"m\":\"não perturbe ☕\"}"
    );
}

#[tokio::test]
async fn limit_is_exact() {
    let ok = format!("\"{}\"\n", "x".repeat(98)); // 100 bytes + \n
    let mut r = reader_over(vec![ok.into_bytes()], 100).await;
    assert_eq!(r.next_line().await.unwrap().unwrap().len(), 100);
    let big = format!("\"{}\"\n", "x".repeat(99));
    let mut r = reader_over(vec![big.into_bytes()], 100).await;
    assert!(matches!(r.next_line().await, Err(FrameError::TooLarge)));
}

#[tokio::test]
async fn huge_line_without_newline_does_not_buffer_forever() {
    let mut r = reader_over(vec![vec![b'x'; 300_000]], 1000).await;
    assert!(matches!(r.next_line().await, Err(FrameError::TooLarge)));
}

#[tokio::test]
async fn invalid_utf8_skips_only_that_line() {
    let mut r = reader_over(vec![b"\xff\xfe\n{}\n".to_vec()], 100).await;
    assert!(matches!(r.next_line().await, Err(FrameError::InvalidUtf8)));
    assert_eq!(r.next_line().await.unwrap().unwrap(), "{}");
}

#[tokio::test]
async fn trailing_fragment_at_eof_is_dropped() {
    let mut r = reader_over(vec![b"{}\n{\"meio".to_vec()], 100).await;
    assert_eq!(r.next_line().await.unwrap().unwrap(), "{}");
    assert!(r.next_line().await.unwrap().is_none());
}

#[tokio::test]
async fn write_message_round_trips_through_line_reader() {
    use lucerna_protocol::{Id, Message, Request};
    let msg = Message::Request(Request {
        id: Id::Num(7),
        method: "bar.set".into(),
        params: Some(serde_json::json!({"text": "linha 1\nlinha 2"})),
    });
    let (mut a, b) = tokio::io::duplex(64);
    let sent = msg.clone();
    tokio::spawn(async move {
        write_message(&mut a, &sent).await.unwrap();
    });
    let mut r = LineReader::new(b, 1024);
    let line = r.next_line().await.unwrap().unwrap();
    assert!(!line.contains('\n'));
    assert_eq!(Message::parse(&line).unwrap(), msg);
    assert!(r.next_line().await.unwrap().is_none());
}

#[tokio::test]
async fn crlf_at_exact_limit_with_cr_in_separate_chunk() {
    let body = format!("\"{}\"", "x".repeat(98)); // 100 bytes
    let mut r = reader_over(
        vec![body.clone().into_bytes(), b"\r".to_vec(), b"\n".to_vec()],
        100,
    )
    .await;
    assert_eq!(r.next_line().await.unwrap().unwrap(), body);
    assert!(r.next_line().await.unwrap().is_none());
}

#[tokio::test]
async fn resumes_after_timeout_mid_line() {
    use std::time::Duration;
    let (mut a, b) = tokio::io::duplex(64);
    let mut r = LineReader::new(b, 1024);
    a.write_all(b"{\"a\":").await.unwrap();
    let cut = tokio::time::timeout(Duration::from_millis(20), r.next_line()).await;
    assert!(cut.is_err(), "a leitura devia esperar o resto da linha");
    a.write_all(b"1}\n").await.unwrap();
    drop(a);
    assert_eq!(r.next_line().await.unwrap().unwrap(), r#"{"a":1}"#);
    assert!(r.next_line().await.unwrap().is_none());
}
