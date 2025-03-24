use ironrdp_mstsgu::GwClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {   
    env_logger::init();

    let target = ironrdp_mstsgu::GwConnectTarget {
        gw_endpoint: "gw:443".to_string(),
        gw_user: "x".to_string(),
        gw_pass: "y".to_string(),
        server: "server".to_string(),
    };

    let conn = GwClient::connect(&target).await.unwrap();
    let mut cl = GwClient::connect_ws(target, conn).await.unwrap();

    let listener = tokio::net::TcpListener::bind("localhost:3389").await?;
    let (mut conn, addr) = listener.accept().await?;
    println!("Got conn");

    tokio::io::copy_bidirectional(&mut conn, &mut cl).await.unwrap();
    Ok(())
}
