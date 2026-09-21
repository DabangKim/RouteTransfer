use std::{
    io::{self, BufRead, Write},
    path::Path,
};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::env::args()
        .nth(1)
        .expect("isolated data directory required");
    let core = routetransfer_lib::core::Core::open(Path::new(&dir))?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(s) => {
                    if tx.blocking_send(s).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    while let Some(s) = rx.recv().await {
        let v: serde_json::Value = serde_json::from_str(&s)?;
        let result = core
            .dispatch(v["action"].as_str().unwrap_or(""), v["args"].clone())
            .await;
        let response = match result {
            Ok(value) => serde_json::json!({"ok":value}),
            Err(e) => serde_json::json!({"error":format!("{e:#}")}),
        };
        println!("{response}");
        io::stdout().flush()?;
    }
    Ok(())
}
