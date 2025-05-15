# Tools JSONRPC over TCP

## Command line

Run and Call a server with that tool.

1. Start a server


```
$ jsonrpc_tool run --addr 0.0.0.0:1234 --path ./scripts
Starting server on address '0.0.0.0:1234'
register echo method
```

In `./scripts` there is some scripts but it can also be a
python scripts or a binary. In the example there is `echo`
which contains the following lines:

```
#! /bin/sh
echo $1
```

Don't forget to make that file executable for the user running the
server.


2. Call the server

```
$ jsonrpc_tool call --addr 0.0.0.0:1234 --method echo --params "hey ho!"
read buffer: {"id":"1","jsonrpc":"2.0","result":"hey hooo!\n"}
Result: hey hooo!
```

## Library

Use also the repo as a rust library:

```rust

use ljsonrpc_over_tcp::*;

println!("Starting server on address '{addr}'");
let mut server = Server::new(addr);
let files = fs::read_dir(path).expect("Unable to read given path");
for file in files {
    let file_path = file.unwrap().path();
    let file_name = file_path.file_name().unwrap().to_str().unwrap();
    let script = file_path.to_str().unwrap().to_string().clone();
    println!("Register {} method", file_name);
    server.add_method(file_name, Arc::new(move |params| exec_cmd(&script, params)));
}
server.run().unwrap();
server.block_on();
```
