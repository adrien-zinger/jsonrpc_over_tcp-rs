use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Error, ErrorKind, Read, Result, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream as TTcpStream};
use tokio::runtime::*;
use tokio::signal;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

type Method = Arc<dyn Fn(String) -> Result<Option<String>> + Send + Sync>;

struct IoCodec {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl IoCodec {
    pub fn new(stream: TcpStream) -> Result<Self> {
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        let writer = BufWriter::new(stream.try_clone()?);
        let reader = BufReader::new(stream);
        Ok(Self { reader, writer })
    }

    pub fn send(&mut self, msg: Value) -> Result<()> {
        self.writer.write_all(msg.to_string().as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn read(&mut self) -> Result<String> {
        let mut buffer = String::new();
        self.reader.read_to_string(&mut buffer)?;
        println!("read buffer: {buffer}");
        Ok(buffer)
    }
}

struct TokioCodec {
    stream: TTcpStream,
}

impl TokioCodec {
    pub fn new(stream: TTcpStream) -> Self {
        Self { stream }
    }

    pub async fn send(&mut self, msg: Value) {
        if let Err(err) = timeout(
            Duration::from_secs(1),
            self.stream.write(msg.to_string().as_bytes()),
        )
        .await
        {
            eprintln!("stream error: failed to respond to stream");
            eprintln!("{:?}", err);
        }
        if let Err(err) = timeout(Duration::from_secs(1), self.stream.flush()).await {
            eprintln!("stream error: failed to flush stream");
            eprintln!("{:?}", err);
        }
    }

    pub async fn read(&mut self) -> Result<String> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut reader = BufReader::new(&mut self.stream);
        let received: Vec<u8> = timeout(Duration::from_secs(1), reader.fill_buf())
            .await??
            .to_vec();
        reader.consume(received.len());
        String::from_utf8(received)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "cannot parse received data"))
    }
}

pub struct Server {
    addr: String,
    runtime: Option<Runtime>,
    token: Option<CancellationToken>,
    methods: HashMap<String, Method>,
}

#[derive(Deserialize)]
struct Request {
    jsonrpc: String,
    id: String,
    method: String,
    params: String,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Deserialize)]
struct Response {
    jsonrpc: String,
    id: String,
    error: Option<JsonRpcError>,
    result: Option<String>,
}

impl Server {
    /** Create a new JSONRPC over TCP server */
    pub fn new(addr: String) -> Self {
        Server {
            addr,
            token: None,
            runtime: None,
            methods: HashMap::default(),
        }
    }

    pub fn add_method(&mut self, name: &str, method: Method) {
        self.methods.insert(name.to_owned(), method);
    }

    pub fn run(&mut self) -> Result<()> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("dh-pub")
            .enable_all()
            .build()?;

        let addr = self.addr.clone();
        let methods = self.methods.clone();
        let token = CancellationToken::new();
        let cloned_token = token.clone();

        runtime.spawn(async move {
            // todo return a clean error instead of unwrap
            let listener = TcpListener::bind(addr.clone()).await.unwrap();
            loop {
                tokio::select! {
                    _ = signal::ctrl_c() => { break; },
                    conn = listener.accept() => {
                        println!("accept entry");
                        handle(conn, &methods).await;
                    }
                };
            }
            println!("Server stopped");
            cloned_token.cancel();
        });

        self.runtime = Some(runtime);
        self.token = Some(token);
        Ok(())
    }

    pub fn block_on(self) {
        if let Some(runtime) = self.runtime {
            runtime.block_on(async move {
                self.token.unwrap().cancelled().await;
            });
            println!("Exit");
        }
    }
}

pub fn send(addr: &str, id: &str, method: &str, params: &str) -> Result<String> {
    let mut codec = IoCodec::new(TcpStream::connect(addr)?)?;

    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    codec.send(body)?;
    let message = codec.read()?;

    if let Ok(res) = serde_json::from_str::<Response>(&message) {
        if res.jsonrpc != "2.0" {
            eprintln!("jsonrpc: invalid version");
        }
        if let Some(result) = res.result {
            return Ok(result);
        }
        if let Some(err) = res.error {
            eprintln!("jsonrpc: id({}) {} ({})", res.id, err.code, err.message);
            Err(Error::new(ErrorKind::Other, "json rpc error raised"))
        } else {
            eprintln!("jsonrpc: id({}) invalid response body", res.id);
            Err(Error::new(ErrorKind::InvalidData, "failed to parse result"))
        }
    } else {
        eprintln!("jsonrpc: id(not) invalid response body");
        Err(Error::new(ErrorKind::InvalidData, "failed to parse result"))
    }
}

async fn handle(conn: Result<(TTcpStream, SocketAddr)>, methods: &HashMap<String, Method>) {
    if let Ok((stream, _addr)) = conn {
        let mut codec = TokioCodec::new(stream);

        let message = match codec.read().await {
            Ok(message) => message,
            Err(err) => {
                eprintln!("stream error: failed to read stream");
                eprintln!("{:?}", err);
                return;
            }
        };

        let request = if let Ok(request) = serde_json::from_str::<Request>(&message) {
            request
        } else {
            eprintln!("error: failed to parse message {message}");
            return codec
                .send(json!({
                   "jsonrpc": "2.0",
                   "error": {
                       "code": -32600i32,
                       "message": "Invalid jsonrpc request",
                   },
                   "id": "-1"
                }))
                .await;
        };

        println!("received request {}", request.method);

        if request.jsonrpc != "2.0" {
            eprintln!("wrong jsonrpc version received");
            return codec
                .send(json!({
                    "jsonrpc": "2.0",
                    "result": "Invalid jsonrpc version",
                    "id": request.id
                }))
                .await;
        }

        if let Some(method) = methods.get(&request.method) {
            if let Ok(res) = method(request.params) {
                println!("method called");
                if let Some(res) = res {
                    codec
                        .send(json!({
                            "jsonrpc": "2.0",
                            "result": res,
                            "id": request.id
                        }))
                        .await;
                } else {
                    codec
                        .send(json!({
                            "jsonrpc": "2.0",
                            "result": "",
                            "id": request.id
                        }))
                        .await;
                }
            } else {
                codec
                    .send(json!({
                        "jsonrpc" : "2.0",
                        "error": {
                            "code": -32603i32,
                            "message": "internal call error"
                        },
                        "id": "-1"
                    }))
                    .await;
            }
        } else {
            codec
                .send(json!({
                     "jsonrpc": "2.0",
                     "error": {
                         "code": -32601i32,
                         "message": "Method not found",
                     },
                     "id": request.id
                }))
                .await;
        }
    } else {
        eprintln!("failed to open connection on server side");
    }
}
