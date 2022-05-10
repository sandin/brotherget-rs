#[macro_use] 
use std::error::Error;
use std::str::FromStr;
use std::path::{Path};
use clap::{Arg, App};
use tokio::fs::{File};
use tokio::sync::mpsc;
use tokio::io::{self, AsyncBufReadExt};
use futures::StreamExt;
use libp2p::{
    swarm::NetworkBehaviour,
    core::upgrade,
    core::ConnectedPoint,
    core::multiaddr::{Protocol::P2p, multihash::Multihash},
    identify::{Identify, IdentifyConfig, IdentifyEvent},
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
use crate::event::{Event, EventBus};

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "OutEvent")]
struct MyBehaviour {
    kademlia: Kademlia<MemoryStore>,
    //mdns: Mdns,
    identity: Identify, // see: https://github.com/libp2p/rust-libp2p/discussions/2447
}

#[derive(Debug)]
enum OutEvent {
    Identify(IdentifyEvent),
    Kademlia(KademliaEvent),
}

impl From<IdentifyEvent> for OutEvent {
    fn from(v: IdentifyEvent) -> Self {
        Self::Identify(v)
    }
}

impl From<KademliaEvent> for OutEvent {
    fn from(v: KademliaEvent) -> Self {
        Self::Kademlia(v)
    }
}

pub async fn join_p2p(keyfile: Option<String>, port: u32, bootnodes: Vec<String>, event_bus: EventBus) -> Result<(), Box<dyn Error>> {
    let mut is_boot_node: bool = false;

    let id_keys = match keyfile {
        Some(filename) => { // use RSA key file
            if Path::new(&filename).exists() {
                is_boot_node = true;
                let mut bytes = std::fs::read(&filename).unwrap();
                match identity::Keypair::rsa_from_pkcs8(&mut bytes) {
                    Ok(r) => r,
                    Err(_) => panic!("bad key file {}", &filename)
                }
            } else {
                panic!("key file {} is not exists", &filename)
            }
        },
        None => identity::Keypair::generate_ed25519() // Create a random PeerId
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

    // Create a swarm to manage peers and events.
    let mut swarm = {
        // Create a Kademlia behaviour.
        let store = MemoryStore::new(local_peer_id);
        let kademlia = Kademlia::new(local_peer_id, store);
        let identity = Identify::new(IdentifyConfig::new(
            "/ipfs/0.1.0".into(),
            id_keys.public(),
        ));
        let behaviour = MyBehaviour { kademlia, identity };

        SwarmBuilder::new(transport, behaviour, local_peer_id)
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
    let local_port = if is_boot_node { port } else {0};
    swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", local_port).parse()?)?;

    for addr in bootnodes {
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
        swarm.behaviour_mut().kademlia.add_address(&peer_id, multiaddr.clone());
    }
    if !is_boot_node {
        //swarm.behaviour_mut().kademlia.bootstrap()?;
    }

    let mut rx = event_bus.receiver;
    let tx = event_bus.sender;
    //tokio::pin!(rx);

    // Kick it off
    loop {
        tokio::select! {
            line = stdin.next_line() => handle_input_line(&mut swarm.behaviour_mut().kademlia, line.expect("Stdin not to close").unwrap()),
            event = rx.recv() => {
                match event? {
                    Event::StartProviding { key } => {
                        swarm.behaviour_mut().kademlia.start_providing(Key::new(&key)).expect("Failed to start providing key");
                    },
                    Event::StopProviding { key } => {
                        swarm.behaviour_mut().kademlia.stop_providing(&Key::new(&key));
                    },
                    Event::Put { key, value } => {
                        swarm.behaviour_mut().kademlia.put_record(Record {
                            key: Key::new(&key),
                            value: value.as_bytes().to_vec(),
                            publisher: None,
                            expires: None,
                        }, Quorum::One).expect("Failed to store record locally.");
                    },
                }
            },
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Listening on {:?}", address);
                        tx.send(Event::PeerReady {});
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, ..} => {
                    },
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, ..} => {
                        //println!("ConnectionClosed, peer {:?}, endpoint {:?}", peer_id, endpoint);
                    },
                    SwarmEvent::Behaviour(message) => {
                        match message {
                            OutEvent::Identify(event) => {
                                match event {
                                    IdentifyEvent::Received {  peer_id, info } => {
                                        println!("IdentifyEvent::Received, peer_id {:?}, info {:?}", peer_id, info);
                                        if info.protocols.contains(&String::from("/ipfs/kad/1.0.0")) {
                                            for addr in info.listen_addrs {
                                                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                                            }
                                        }
                                    },
                                    IdentifyEvent::Sent {  peer_id } => {
                                        println!("IdentifyEvent::Sent, peer_id {:?}", peer_id);
                                    },
                                    IdentifyEvent::Pushed {  peer_id } => {
                                        println!("IdentifyEvent::Sent, peer_id {:?}", peer_id);
                                    },
                                    IdentifyEvent::Error {  peer_id, error } => {
                                        println!("IdentifyEvent::Sent, peer_id {:?}, error {:?}", peer_id, error);
                                    },
                                    _ => {}
                                }
                            },
                            OutEvent::Kademlia(event) => {
                                match event {
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
                    },
                    _ => {}
                }
            }
        }
    }
    //Ok(())
}

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

    let key_file: String = String::from("private.pk8");
    let bootnode_port: u32 = 53308;
    let bootnodes: Vec<String> = vec![
        String::from("/ip4/127.0.0.1/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa"),
    ];

    let key_file = if matches.is_present("bootnode") { Some(key_file) } else { None };
    let (tx, mut rx) = mpsc::channel(32);
    join_p2p(key_file, bootnode_port, bootnodes.to_vec(), rx).await?;
    Ok(())
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