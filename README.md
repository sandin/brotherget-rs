# Brother Get

BrotherGet is a P2P downloader.

## Usage

```
$ bget https://github.com/seanmonstar/reqwest/archive/refs/tags/v0.11.10.zip --config config.json
```

output:
```
start downloading...                                                                                                    
url: https://golang.google.cn/dl/go1.18.1.src.tar.gz, content_length: 22834149, chunk_count: 5, chunk_length: 4566830, last_chunk_length: 4566829
[eta 00:00:14] ####------------------------------------  452028/4566830 10 % 268.33 KiB/s range=0-4566829
[eta 00:14:09] #---------------------------------------   18663/4566830 0  % 5.23 KiB/s range=4566830-9133659
[eta 00:00:14] ###-------------------------------------  307600/4566830 7  % 285.65 KiB/s range=9133660-13700489
[eta 00:00:13] ###-------------------------------------  325526/4566830 7  % 307.63 KiB/s range=13700490-18267319
[eta 00:00:14] ###-------------------------------------  288252/4566829 6  % 285.70 KiB/s range=18267320-22834148 
```

## Server

start server:
```
$ bget start_server --config config.json
```

## Config

peer node:
```json
{
    "proxy": {
        "password": "mMVlD2/6lni6EX6l5Tx3khJcl7Y=",
    },
    "p2p": {
        "bootnodes": [
            "/ip4/127.0.0.1/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa"
        ]
    }
}
```


boot node:
```json
{
    "proxy": {
        "password": "mMVlD2/6lni6EX6l5Tx3khJcl7Y=",
    },
    "p2p": {
        "peer_port": 53308,          
        "key_file": "private.pk8",
        "bootnodes": [
            "/ip4/127.0.0.1/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa"
        ]
    }
}
```

            