use std::net::TcpListener;
use std::io::{Read, Write};
use toml;
use std::fs::File;
use serde::Deserialize;
use std::thread;
use chrono::Local;

#[derive(Deserialize, Clone)]
struct Server {
    name: String,
    config: String,
}

#[derive(Deserialize)]
struct Config {
    servers: Vec<Server>,
}

#[derive(Deserialize, Clone)]
struct ServerConfig {
    server: ServerInfo,
    #[serde(rename = "type")]
    server_type: TypeInfo,
    #[serde(rename = "static", default)]
    static_config: Option<StaticConfig>,
}

#[derive(Deserialize, Clone)]
struct ServerInfo {
    address: String,
    port: u16,
}

#[derive(Deserialize, Clone)]
struct TypeInfo {
    name: String,
}

#[derive(Deserialize, Clone)]
struct StaticConfig {
    webroot: String,
    index: String,
}

/// 加载并解析TOML配置文件
fn load_config(path: &str) -> Config {
    let mut config_file = File::open(path).expect("无法打开配置文件");
    let mut config_contents = String::new();
    config_file.read_to_string(&mut config_contents).expect("无法读取配置文件");
    toml::from_str(&config_contents).expect("无法解析配置文件")
}

/// 加载并解析服务器配置
fn load_server_config(path: &str) -> ServerConfig {
    let mut server_file = File::open(path).expect("无法打开服务器配置文件");
    let mut server_contents = String::new();
    server_file.read_to_string(&mut server_contents).expect("无法读取服务器配置文件");
    toml::from_str(&server_contents).expect("无法解析服务器配置文件")
}

/// 从请求中提取路径
fn extract_path(buffer: &[u8]) -> String {
    match buffer.iter().position(|&b| b == b' ') {
        Some(index) => {
            // 找到第二个空格
            match buffer[index+1..].iter().position(|&b| b == b' ') {
                Some(second_space) => {
                    let path = String::from_utf8_lossy(&buffer[index+1..index+1+second_space]).to_string();
                    path.trim().to_string()
                }
                None => String::from("/")
            }
            
        }
        None => String::from("/")
    }
}

/// 记录访问日志
fn log_access(client_addr: &str, path: &str, status_code: u16) {
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
    println!("[{}] {} - {} - {}", timestamp, client_addr, path, status_code);
}

/// 处理静态文件请求
fn handle_static_request(static_config: &StaticConfig, path: &str) -> String {
    // 如果路径为/，则返回index文件
    let actual_path = if path == "/" {
        &static_config.index
    } else {
        path
    };
    
    let file_path = format!("{}/{}", static_config.webroot, actual_path);
    match File::open(&file_path) {
        Ok(mut file) => {
            let mut contents = String::new();
            match file.read_to_string(&mut contents) {
                Ok(_) => {
                    let mut response = String::from("HTTP/1.1 200 OK\r\n");
                    response.push_str("Server: nextWeb 0.1.0\r\n");
                    response.push_str("Content-Type: text/html; charset=utf-8\r\n");
                    response.push_str("\r\n");
                    response.push_str(&contents);
                    response
                }
                Err(_) => {
                    String::from("HTTP/1.1 500 Internal Server Error\r\n\r\n500 Internal Server Error")
                }
            }
        }
        Err(_) => {
            String::from("HTTP/1.1 404 Not Found\r\n\r\n404 Not Found")
        }
    }
}

/// 处理客户端请求
fn handle_client(stream: &mut std::net::TcpStream, server_config: &ServerConfig) {
    let client_addr = match stream.peer_addr() {
        Ok(addr) => addr.to_string(),
        Err(_) => String::from("unknown")
    };
    
    let mut buffer = [0; 1024];
    if let Err(_) = stream.read(&mut buffer) {
        log_access(&client_addr, "-", 400);
        return;
    }
    
    let path = extract_path(&buffer);
    
    let response = match server_config.server_type.name.as_str() {
        "static" => {
            match &server_config.static_config {
                Some(static_config) => handle_static_request(static_config, &path),
                None => String::from("HTTP/1.1 500 Internal Server Error\r\n\r\n500 Internal Server Error: Static configuration is missing")
            }
        }
        _ => String::from("HTTP/1.1 501 Not Implemented\r\n\r\n501 Not Implemented")
    };
    
    // 从响应中提取状态码
    let status_code = if response.starts_with("HTTP/1.1 200") {
        200
    } else if response.starts_with("HTTP/1.1 404") {
        404
    } else if response.starts_with("HTTP/1.1 500") {
        500
    } else if response.starts_with("HTTP/1.1 501") {
        501
    } else {
        0
    };
    
    log_access(&client_addr, &path, status_code);
    send_response(stream, &response);
}

/// 发送HTTP响应
fn send_response(stream: &mut std::net::TcpStream, response: &str) {
    let _ = stream.write(response.as_bytes());
}

/// 启动服务器
fn start_server(server: Server) {
    let server_config = load_server_config(&server.config);
    let address = format!("{}:{}", server_config.server.address, server_config.server.port);
    let listener = TcpListener::bind(&address).expect("无法绑定端口");
    println!("服务器 '{}' 监听于 {}", server.name, address);
    
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                handle_client(&mut stream, &server_config);
            }
            Err(e) => {
                eprintln!("接受连接失败: {}", e);
            }
        }
    }
}

fn main() {
    println!("nextWeb 0.1.0");
    
    let config = load_config("config.toml");
    
    let mut handles = vec![];
    
    for server in config.servers {
        let handle = thread::spawn(move || {
            start_server(server);
        });
        handles.push(handle);
    }
    
    // 等待所有线程完成
    for handle in handles {
        handle.join().unwrap();
    }
}
