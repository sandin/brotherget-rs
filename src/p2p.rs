use std::error::Error;
use std::str::FromStr;
use std::path::{Path};
use clap::{Arg, App};
use tokio::fs::{File};
use tokio::io::{self, AsyncBufReadExt};
use futures::StreamExt;
use libp2p::{
    swarm::NetworkBehaviour,
    core::upgrade,
    core::ConnectedPoint,
    core::multiaddr::{Protocol::P2p, multihash::Multihash},
    kad::{record::store::MemoryStore, record::Key, AddProviderOk, Kademlia, KademliaEvent, PeerRecord, PutRecordOk, QueryResult, Quorum, Record},
    identity,
    mdns::{Mdns, MdnsEvent},
    mplex,
    noise,
    swarm::{dial_opts::DialOpts, NetworkBehaviourEventProcess, SwarmBuilder, SwarmEvent},
    // `TokioTcpConfig` is available through the `tcp-tokio` feature.
    tcp::TokioTcpConfig,
    Multiaddr,
    NetworkBehaviour,
    PeerId,
    Transport,
};
#[macro_use] extern crate log;
extern crate env_logger;

const KEY_FILE: &'static str = "private.pk8";
const BOOTNODE_PORT: u32 = 53308;
const BOOTNODES: [&'static str; 1] = [
    "/ip4/192.168.2.7/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa",
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();

    let matches  = App::new("bp2p")
    .version(env!("CARGO_PKG_VERSION"))
    .arg(
      Arg::with_name("bootnode")
          .short("b")
          .long("bootnode")
          .takes_value(false)
          .help("is bootnode"),
    )
    .arg(
      Arg::with_name("peer").required(false).help("peer id")
    )
    .get_matches();

    let is_boot_node: bool = matches.is_present("bootnode");

    let id_keys = {
        if is_boot_node && Path::new(KEY_FILE).exists() {
            // use RSA keys
            let mut bytes = std::fs::read("private.pk8").unwrap();
            match identity::Keypair::rsa_from_pkcs8(&mut bytes) {
                Ok(r) => r,
                Err(_) => panic!("bad key file")
            }
        } else {
            // Create a random PeerId
            identity::Keypair::generate_ed25519()
        }
    };
    let local_peer_id = PeerId::from(id_keys.public());
    println!("Local peer id: {:?}", local_peer_id);

    // Create a keypair for authenticated encryption of the transport.
    let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
        .into_authentic(&id_keys)
        .expect("Signing libp2p-noise static DH keypair failed.");

    // Create a tokio-based TCP transport use noise for authenticated
    // encryption and Mplex for multiplexing of substreams on a TCP stream.
    let transport = TokioTcpConfig::new()
        .nodelay(true)
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    // Create a Kademlia behaviour.
    let store = MemoryStore::new(local_peer_id);
    let kademlia = Kademlia::new(local_peer_id, store);

    // Create a swarm to manage peers and events.
    let mut swarm = {
        SwarmBuilder::new(transport, kademlia, local_peer_id)
        // We want the connection background tasks to be spawned
        // onto the tokio runtime.
        .executor(Box::new(|fut| {
            tokio::spawn(fut);
        }))
        .build()
    };

    // Read full lines from stdin
    let mut stdin = io::BufReader::new(io::stdin()).lines();

    // Listen on all interfaces and whatever port the OS assigns
    let local_port = if is_boot_node { BOOTNODE_PORT } else {0};
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", local_port).parse()?)?;

    for addr in &BOOTNODES {
        let mut multiaddr = Multiaddr::from_str(&addr)?;
        let hash: Multihash = match multiaddr.pop().unwrap() {
            P2p(m) => m,
            _ => panic!("bad boot node multiaddr") 
        };
        let peer_id: PeerId = match PeerId::from_multihash(hash) {
            Ok(p) => p,
            Err(h) => panic!("bad boot node multiaddr hash") ,
        };
        if peer_id == local_peer_id {
            println!("is bootnode, addr {}", &addr);
            continue; // DO NOT ADD MYSELF
        }
        println!("add boot addr {}", &addr);
        swarm.behaviour_mut().add_address(&peer_id, multiaddr.clone());
    }
    if !is_boot_node {
        swarm.behaviour_mut().bootstrap()?;
    }

    // Kick it off
    loop {
        tokio::select! {
            line = stdin.next_line() => handle_input_line(&mut swarm.behaviour_mut(), line.expect("Stdin not to close").unwrap()),
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Listening on {:?}", address);
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, ..} => {
                        //if is_boot_node {
                        /*
                        match endpoint {
                            ConnectedPoint::Dialer { .. } => {},
                            ConnectedPoint::Listener { local_addr, send_back_addr } => { 
                                println!("ConnectionEstablished, add_address peer {:?}, endpoint {:?}", peer_id, &send_back_addr);
                                swarm.behaviour_mut().add_address(&peer_id, send_back_addr.clone()); 
                            },
                        }
                        */
                        //}
                        // FIXME: need to get addr of peer_id, see: https://github.com/libp2p/rust-libp2p/discussions/2447
                    },
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, ..} => {
                        //println!("ConnectionClosed, peer {:?}, endpoint {:?}", peer_id, endpoint);
                    },
                    SwarmEvent::Behaviour(message) => {
                        match message {
                            KademliaEvent::RoutingUpdated { peer, is_new_peer, addresses, .. } => {
                                println!("RoutingUpdated, peer {:?}, is_new_peer {:?}, addresses {:#?}", peer, is_new_peer, addresses);
                            },
                            KademliaEvent::UnroutablePeer { peer } => {
                                println!("UnroutablePeer, peer {:?}", peer);
                            },
                            KademliaEvent::RoutablePeer { peer, address } => {
                                println!("RoutablePeer, peer {:?}, address {:?}", peer, address);
                            },
                            KademliaEvent::PendingRoutablePeer { peer, address } => {
                                println!("PendingRoutablePeer, peer {:?}, address {:?}", peer, address);
                            },

                            KademliaEvent::OutboundQueryCompleted { result, .. } => match result {
                                QueryResult::GetProviders(Ok(ok)) => {
                                    for peer in ok.providers {
                                        println!(
                                            "Peer {:?} provides key {:?}",
                                            peer,
                                            std::str::from_utf8(ok.key.as_ref()).unwrap()
                                        );
                                    }
                                }
                                QueryResult::GetProviders(Err(err)) => {
                                    eprintln!("Failed to get providers: {:?}", err);
                                }
                                QueryResult::GetRecord(Ok(ok)) => {
                                    for PeerRecord {
                                        record: Record { key, value, .. },
                                        ..
                                    } in ok.records
                                    {
                                        println!(
                                            "Got record {:?} {:?}",
                                            std::str::from_utf8(key.as_ref()).unwrap(),
                                            std::str::from_utf8(&value).unwrap(),
                                        );
                                    }
                                }
                                QueryResult::GetRecord(Err(err)) => {
                                    eprintln!("Failed to get record: {:?}", err);
                                }
                                QueryResult::PutRecord(Ok(PutRecordOk { key })) => {
                                    println!(
                                        "Successfully put record {:?}",
                                        std::str::from_utf8(key.as_ref()).unwrap()
                                    );
                                }
                                QueryResult::PutRecord(Err(err)) => {
                                    eprintln!("Failed to put record: {:?}", err);
                                }
                                QueryResult::StartProviding(Ok(AddProviderOk { key })) => {
                                    println!(
                                        "Successfully put provider record {:?}",
                                        std::str::from_utf8(key.as_ref()).unwrap()
                                    );
                                }
                                QueryResult::StartProviding(Err(err)) => {
                                    eprintln!("Failed to put provider record: {:?}", err);
                                }
                                _ => {}
                            },
                            _ => {}
                        }
                    },
                    _ => {}
                }
            }
        }
    }
}


fn handle_input_line(kademlia: &mut Kademlia<MemoryStore>, line: String) {
    let mut args = line.split(' ');

    match args.next() {
        Some("GET") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        eprintln!("Expected key");
                        return;
                    }
                }
            };
            kademlia.get_record(key, Quorum::One);
        }
        Some("GET_PROVIDERS") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        eprintln!("Expected key");
                        return;
                    }
                }
            };
            kademlia.get_providers(key);
        }
        Some("PUT") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        eprintln!("Expected key");
                        return;
                    }
                }
            };
            let value = {
                match args.next() {
                    Some(value) => value.as_bytes().to_vec(),
                    None => {
                        eprintln!("Expected value");
                        return;
                    }
                }
            };
            let record = Record {
                key,
                value,
                publisher: None,
                expires: None,
            };
            kademlia
                .put_record(record, Quorum::One)
                .expect("Failed to store record locally.");
        }
        Some("PUT_PROVIDER") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        eprintln!("Expected key");
                        return;
                    }
                }
            };

            kademlia
                .start_providing(key)
                .expect("Failed to start providing key");
        }
        Some("STOP_PROVIDER") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        eprintln!("Expected key");
                        return;
                    }
                }
            };

            kademlia.stop_providing(&key);
        }
        _ => {
            eprintln!("expected GET, GET_PROVIDERS, PUT or PUT_PROVIDER");
        }
    }
}