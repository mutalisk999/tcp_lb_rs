use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};
use std::error::Error;

use crate::proxy::config::Config;
use std::collections::HashMap;
use tokio::sync::Mutex;
use std::sync::Arc;
use crate::proxy::connection::{NodeConnection, TargetConnection};
use crate::proxy::target::{Target, dump_targets};
use tokio::time::Duration;

pub async fn start_tcp_proxy(config: &Config, targets: Arc<Mutex<HashMap<String, Target>>>,
                             conn_pair_n2t: Arc<Mutex<HashMap<Arc<Mutex<NodeConnection>>, Arc<Mutex<TargetConnection>>>>>,
                             conn_pair_t2n: Arc<Mutex<HashMap<Arc<Mutex<TargetConnection>>, Arc<Mutex<NodeConnection>>>>>
    ) -> Result<(), Box<dyn Error>>{

    let node_listener = TcpListener::bind(config.lb_node.listen.as_str())
        .await.expect(format!("Failure binding node listen endpoint [{}]", config.lb_node.listen).as_str());

    loop {
        let (mut tcp_stream_accept, remote_addr) = node_listener.accept().await?;
        println!("remote connection from {}", remote_addr);

        let targets_dump = dump_targets(targets.clone(), conn_pair_t2n.clone()).await;
        let mut socket_conn: Option<TcpSocket> = None;
        let mut tcp_stream_conn: Option<TcpStream> = None;
        let mut conn_target_id: Option<String> = None;
        let mut target_time_out: Option<u32> = None;

        for t in targets_dump.iter() {
            if !t.target.target_active {
                continue;
            }
            if t.target_conn_count > t.target.target_max_conn {
                continue;
            }

            let r = TcpSocket::new_v4();
            socket_conn = match r {
                Ok(s) => Some(s),
                Err(_) => continue
            };
            let r = socket_conn.unwrap().connect(t.target.target_endpoint.parse().unwrap()).await;
            tcp_stream_conn = match r {
                Ok(c) => Some(c),
                Err(_) => continue
            };

            conn_target_id = Some(t.target.target_endpoint.clone());
            target_time_out = Some(t.target.target_timeout);
            break;
        }

        match tcp_stream_conn {
            Some(_) => (),
            None => {
                let _ = tcp_stream_accept.shutdown().await;
                continue;
            }
        }
        let conn_target_id = conn_target_id.unwrap();
        let mut tcp_stream_conn = tcp_stream_conn.unwrap();

        let accept_connection = Arc::new(Mutex::new(NodeConnection::new(tcp_stream_accept)));
        let conn_connection  = Arc::new(Mutex::new(TargetConnection::new(tcp_stream_conn, conn_target_id)));

        let accept_connection_t1 = accept_connection.clone();
        let conn_connection_t1 = conn_connection.clone();
        let accept_connection_t2 = accept_connection.clone();
        let conn_connection_t2 = conn_connection.clone();

        tokio::spawn(async move {
            let mut buf = [0; 1024];
            loop {
                println!("aaa in");
                let n = match accept_connection_t1.lock().await.connection.tcp_stream.read(&mut buf).await {
                    Ok(n) if n == 0 => return,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("failed to read from socket; err = {:?}", e);
                        return;
                    }
                };
                println!("aaa: read {}", n);

                if let Err(e) = conn_connection_t1.lock().await.connection.tcp_stream.write_all(&buf[0..n]).await {
                    eprintln!("failed to write to socket; err = {:?}", e);
                    return;
                }
                println!("aaa: write");
            }
        });

        tokio::spawn(async move {
            let mut buf = [0; 1024];
            loop {
                println!("bbb in");
                let n = match conn_connection_t2.lock().await.connection.tcp_stream.read(&mut buf).await {
                    Ok(n) if n == 0 => return,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("failed to read from socket; err = {:?}", e);
                        return;
                    }
                };
                println!("bbb: read {}", n);

                if let Err(e) = accept_connection_t2.lock().await.connection.tcp_stream.write_all(&buf[0..n]).await {
                    eprintln!("failed to write to socket; err = {:?}", e);
                    return;
                }
                println!("bbb: write");
            }
        });
    }
}
