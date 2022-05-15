# Brother Get

BrotherGet is a P2P downloader.

# Requirement

* [protobuf](https://grpc.io/docs/protoc-installation/)

install protobuf on macOS:
```
$ brew install protobuf
```

## Build

```
$ cargo build --release
```

## Bootstrap


We must hava at least one boot node to bootstrap the P2P network:

**Step 1: Generating RSA keys with OpenSSL:**
```
openssl genrsa -out private.pem 2048
openssl pkcs8 -in private.pem -inform PEM -topk8 -out private.pk8 -outform DER -nocrypt
rm private.pem      # optional
```

**Step 2: Create config file for boot node:**
```json
{
    "proxy": {
        "password": "mMVlD2/6lni6EX6l5Tx3khJcl7Y=",
    },
    "p2p": {
        "peer_port": 53308,          
        "key_file": "private_UUxa.pk8",
        "bootnodes": [
            "/ip4/127.0.0.1/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa",
            "/ip4/127.0.0.1/tcp/53309/p2p/QmckweETKF3ncFQRYYwbbBHZRH6RhqPkG3sCjF5E68DkP9"
        ]
    }
}
```
> NOTE: change `key_file` to your key file. 


**Step 3: Run P2P node:**
```
$ bget start_server --config config_boot1.json
```


## Download

**Step 1: Create config file for normal node:**
```json
{
    "proxy": {
        "password": "mMVlD2/6lni6EX6l5Tx3khJcl7Y=",
    },
    "p2p": {
        "bootnodes": [
            "/ip4/127.0.0.1/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa",
            "/ip4/127.0.0.1/tcp/53309/p2p/QmckweETKF3ncFQRYYwbbBHZRH6RhqPkG3sCjF5E68DkP9"
        ]
    }
}
```
> NOTE: change `bootnodes` to your nodes(see the console output of bootnode). 


**Step 2: Download file:**
```
$ bget https://github.com/seanmonstar/reqwest/archive/refs/tags/v0.11.10.zip --config config.json
```

Output:
```
start downloading...                                                                                                    
url: https://golang.google.cn/dl/go1.18.1.src.tar.gz, content_length: 22834149, chunk_count: 5, chunk_length: 4566830, last_chunk_length: 4566829
[eta 00:00:14] ####------------------------------------  452028/4566830 10 % 268.33 KiB/s range=0-4566829
[eta 00:14:09] #---------------------------------------   18663/4566830 0  % 5.23 KiB/s range=4566830-9133659
[eta 00:00:14] ###-------------------------------------  307600/4566830 7  % 285.65 KiB/s range=9133660-13700489
[eta 00:00:13] ###-------------------------------------  325526/4566830 7  % 307.63 KiB/s range=13700490-18267319
[eta 00:00:14] ###-------------------------------------  288252/4566829 6  % 285.70 KiB/s range=18267320-22834148 
```


## Development

You can deploy a set of nodes on the local machine for development:


**Step 1: Start two bootnodes:**
```
cargo run --bin bget -- --config config_boot1.json -v
```

```
cargo run --bin bget -- --config config_boot2.json -v
```

**Step 2: Download file:**
```
cargo run --bin bget https://golang.google.cn/dl/go1.18.1.src.tar.gz --config config.json
```