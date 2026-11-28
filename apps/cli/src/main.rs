//! `mocky-mail`: Mocky Mail from the command line, and its MCP server.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(mocky_mail_cli::run(args)),
        Err(error) => {
            eprintln!("mocky-mail: {error}");
            1
        }
    };
    std::process::exit(code);
}
