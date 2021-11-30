use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use futures::FutureExt;
use memchr::memmem;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let listen_addr =
        std::env::var("TCP_PROXY_LISTEN").unwrap_or_else(|_| "0.0.0.0:3000".to_string());

    let default_addr =
        std::env::var("TCP_PROXY_DEFAULT").unwrap_or_else(|_| "127.0.0.1:3001".to_string());

    let timeout = std::env::var("TCP_PROXY_TIMEOUT")
        .unwrap_or_else(|_| "10000".to_string())
        .parse::<u64>()
        .unwrap();

    let lookahead_size = std::env::var("TCP_PROXY_LOOKAHEAD_SIZE")
        .unwrap_or_else(|_| "16000".to_string())
        .parse::<usize>()
        .unwrap();

    println!("Listening on: {}", listen_addr);

    let listener = TcpListener::bind(listen_addr).await?;

    while let Ok((inbound, addr)) = listener.accept().await {
        println!("Accept from: {}", addr);
        match get_server_addr(&inbound, &default_addr, timeout, lookahead_size).await {
            Ok((server_addr, buffered_data)) => {
                println!("Proxying {} to: {}", addr, server_addr);
                let transfer = transfer(inbound, server_addr.clone(), buffered_data).map(|r| {
                    if let Err(e) = r {
                        println!("Failed to transfer; error={}", e);
                    }
                });

                tokio::spawn(transfer);
            }
            Err(e) => {
                println!("Failed to transfer; error={}", e);
            }
        }
    }

    Ok(())
}

async fn get_server_addr(
    inbound: &TcpStream,
    default: &String,
    timeout: u64,
    lookahead_size: usize,
) -> Result<(String, Vec<u8>), Box<dyn Error>> {
    let mut v: Vec<u8> = Vec::new();
    let sleep = tokio::time::sleep(tokio::time::Duration::from_millis(timeout));
    tokio::pin!(sleep);

    loop {
        tokio::select! {
                    _ = &mut sleep => {
                        println!("timed out after {} milliseconds without reading {} bytes or finding \\r\\n\\r\\n", timeout, lookahead_size);
                        return Ok((default.clone(), v));
                }
                   _ =  inbound.readable() => {
                let mut msg = vec![0; 1024];

                match inbound.try_read(&mut msg) {
                    Ok(n) => {
                        msg.truncate(n);
                        v.append(&mut msg);
                        let m1 = b"\r\n\r\n";
                        if let Some(i) = memmem::find_iter(&v, m1).next() {
                            let s = match std::str::from_utf8(&v[..i + m1.len()]) {
                                Ok(v) => v.to_lowercase(),
                                Err(e) => return Err(format!("Invalid UTF-8 sequence: {}", e).into()),
                            };
                            let m2 = format!("{}:",std::env::var("TCP_PROXY_HEADER").unwrap_or_else(|_| "tcp_forward:".to_string()).to_string());
                            if let Some(j) = s[..i].find(&m2) {
                                if let Some(k) = s[j + m2.len()..i + m1.len()].find("\r\n") {
                                    return Ok((s[j + m2.len()..j + m2.len() + k].trim().to_string(), v));
                                } else {
                                    return Err("found header but not value end".into());
                                }
                            } else {
                                return Ok((default.clone(), v));
                            }
                        } else {
                            if v.len() > lookahead_size {
                                println!("did not find proxy after reading {} bytes", lookahead_size);
                                return Ok((default.clone(), v));
                            }
                        }
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(e) => {
                        return Err(e.into());
                    }
                 }
            }
        }
    }
}

async fn transfer(
    mut inbound: TcpStream,
    proxy_addr: String,
    buffered_data: Vec<u8>,
) -> Result<(), Box<dyn Error>> {
    let mut outbound = TcpStream::connect(proxy_addr).await?;

    outbound.write(&buffered_data).await?;

    let (mut ri, mut wi) = inbound.split();
    let (mut ro, mut wo) = outbound.split();

    let client_to_server = async {
        io::copy(&mut ri, &mut wo).await?;
        wo.shutdown().await
    };

    let server_to_client = async {
        io::copy(&mut ro, &mut wi).await?;
        wi.shutdown().await
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}
