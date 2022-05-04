# Brother Get

An HTTP downloader which can crack the intranet speed limit.

## Usage

```
$ bget https://github.com/seanmonstar/reqwest/archive/refs/tags/v0.11.10.zip --config config.json
```


## Config

```json
{
    "servers": [
        {
            "server": "127.0.0.1",
            "server_port": 8388,
            "password": "password",
            "method": "aes-256-gcm",
            "local_address": "127.0.0.1",
            "local_port": 1080
        },
        {
            "server": "127.0.0.1",
            "server_port": 8389,
            "password": "password",
            "method": "aes-256-gcm",
            "local_address": "127.0.0.1",
            "local_port": 1081
        }
    ]
}

```

            