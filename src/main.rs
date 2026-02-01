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
    #[serde(rename = "proxy", default)]
    proxy_config: Option<ProxyConfig>,
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

#[derive(Deserialize, Clone, Debug)]
struct ProxyConfig {
    backend: String,
    modify_host: bool,
    header_host: String,
    modify_server: bool,
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
    let config: ServerConfig = toml::from_str(&server_contents).expect("无法解析服务器配置文件");
    println!("加载配置文件: {}", path);
    println!("服务器类型: {}", config.server_type.name);
    println!("代理配置: {:?}", config.proxy_config);
    config
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
                    response.push_str("Server: nextWeb/0.1.0\r\n");
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

/// 处理代理请求
fn handle_proxy_request(proxy_config: &ProxyConfig, request: &str) -> String {
    use std::net::TcpStream;
    use std::io::{Read, Write};
    use std::time::Duration;
    
    // 解析后端服务器地址
    let backend_url = proxy_config.backend.trim_start_matches("http://");
    let (backend_host, backend_port_str) = match backend_url.split_once(':') {
        Some((host, port)) => (host, port),
        None => (backend_url, "80"),
    };
    
    let backend_port: u16 = backend_port_str.parse().unwrap_or(80);
    
    // 连接到后端服务器
    let backend_addr = format!("{}:{}", backend_host, backend_port);
    let socket_addr: std::net::SocketAddr = backend_addr.parse().expect("Invalid backend address");
    match TcpStream::connect_timeout(&socket_addr, Duration::from_secs(5)) {
        Ok(mut backend_stream) => {
            // 根据配置修改请求头
            let modified_request = if proxy_config.modify_host {
                // 替换Host头
                let host_header = format!("Host: {}", proxy_config.header_host);
                request.lines()
                    .map(|line| {
                        if line.starts_with("Host:") {
                            host_header.clone()
                        } else {
                            line.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\r\n")
            } else {
                request.to_string()
            };
            
            // 发送请求到后端
            if let Err(_) = backend_stream.write_all(modified_request.as_bytes()) {
                return String::from("HTTP/1.1 502 Bad Gateway\r\n\r\n502 Bad Gateway");
            }
            
            // 读取后端响应
            let mut response_buffer = [0; 8192];
            match backend_stream.read(&mut response_buffer) {
                Ok(bytes_read) => {
                    let mut response = String::from_utf8_lossy(&response_buffer[..bytes_read]).to_string();
                    
                    // 根据配置修改Server头
                    if proxy_config.modify_server {
                        // 提取原始Server头
                        let original_server = response.lines()
                            .find(|line| line.starts_with("Server:"))
                            .map(|line| line.trim_start_matches("Server:").trim().to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        
                        // 构建新的Server头
                        let new_server_header = format!("Server: nextWeb({})/0.1.0", original_server);
                        
                        // 替换Server头
                        response = response.lines()
                            .map(|line| {
                                if line.starts_with("Server:") {
                                    new_server_header.clone()
                                } else {
                                    line.to_string()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\r\n");
                    }
                    
                    response
                }
                Err(_) => {
                    String::from("HTTP/1.1 502 Bad Gateway\r\n\r\n502 Bad Gateway")
                }
            }
        }
        Err(_) => {
            String::from("HTTP/1.1 502 Bad Gateway\r\n\r\n502 Bad Gateway")
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
    
    // 将原始请求转换为字符串
    let request = String::from_utf8_lossy(&buffer).to_string();
    let path = extract_path(&buffer);
    
    let response = match server_config.server_type.name.as_str() {
        "static" => {
            match &server_config.static_config {
                Some(static_config) => handle_static_request(static_config, &path),
                None => String::from("HTTP/1.1 500 Internal Server Error\r\n\r\n500 Internal Server Error: Static configuration is missing")
            }
        }
        "proxy" => {
            match &server_config.proxy_config {
                Some(proxy_config) => handle_proxy_request(proxy_config, &request),
                None => String::from("HTTP/1.1 500 Internal Server Error\r\n\r\n500 Internal Server Error: Proxy configuration is missing")
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
    } else if response.starts_with("HTTP/1.1 502") {
        502
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
