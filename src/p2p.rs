#[macro_use] 
use std::error::Error;
use std::str::FromStr;
use std::path::{Path};
use std::net::Ipv4Addr;
use std::fmt::Debug;
use std::time::{Duration, Instant};
use clap::{Arg, App};
use tokio::fs::{File};
use tokio::sync::oneshot;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tokio::io::{self, AsyncBufReadExt};
use futures::StreamExt;
use libp2p::{
    swarm::NetworkBehaviour,
    core::upgrade,
    core::ConnectedPoint,
    core::multiaddr::{Protocol::P2p,  Protocol, multihash::Multihash},
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
use log;
use env_logger;
use crate::event::{Event, EventBus};
use crate::service::{P2PService, SSProxyService};
use crate::error::BError;

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

pub async fn discover_p2p_services<T>(service_name: String, keyfile: Option<String>, port: u32, bootnodes: Vec<String>, timeout: Duration) -> Result<Vec<T>, Box<dyn Error>> 
    where T: P2PService + Debug + Default
{
    let mut found_services = vec![];  // list of p2p service
    let mut found_providers = vec![]; // list of peer id 
    let mut provider_record_result_count = 0;

    let event_bus = EventBus::new();
    let event_bus1 = event_bus.clone();
    tokio::spawn(async move {
        join_p2p(keyfile, port, bootnodes, event_bus1.clone()).await.unwrap();
        event_bus1.sender.send(Event::PeerStoped).unwrap();
    });

    let timeout_f = sleep(timeout); // future
    let rx = event_bus.receiver;

    tokio::pin!(timeout_f);
    tokio::pin!(rx);

    // STATE MACHINE:
    //         Idle 
    //           |   join_p2p() 
    //           v
    //      PeerStarted
    //           |   post_event(GetProviders) // find peers
    //           v
    //    GetProvidersResult
    //           |   post_event(GetRecord)    // find proxy url of each peer
    //           v
    //     GetRecordResult
    //           |   break                    // got all we need
    //           v
    //       PeerStoped
    loop {
        tokio::select! {
          e = rx.recv() => {
            match e.unwrap() {
              Event::PeerStarted { peer_id, addr, port } => {
                println!("peer is ready, peer_id={}, addr={}, port={}", peer_id, addr, port);
                event_bus.sender.send(Event::GetProviders { key: service_name.clone() }).unwrap();
              },
              Event::PeerStoped => {
                println!("peer/proxy stoped");
                break;
              },
              Event::GetProvidersResult { key, providers } => {
                println!("found providers(key={}): {:#?}", &key, providers);
                if providers.len() > 0 {
                    for provider in providers {
                        found_providers.push(provider.clone());
                        // TODO: ping the peer
                        let service_info_key = format!("{}_{}", &service_name, &provider);
                        event_bus.sender.send(Event::GetRecord { key: service_info_key }).unwrap();
                    }
                } else {
                    println!("found all proxies: {:#?}", found_services);
                    event_bus.sender.send(Event::PeerStoped).unwrap();
                    break; // got all we need
                }
              },
              Event::GetRecordResult { key, value } => {
                provider_record_result_count += 1;
                let mut p2p_service = T::default();
                match p2p_service.deserialize(value) {
                    Ok(()) => {
                        println!("found service: peer_id={}, service={:?}", &key, p2p_service);
                        found_services.push(p2p_service);
                    },
                    Err(e) => {
                        println!("found service: peer_id={}, error={}", &key, e.to_string());
                    }
                }

                if found_providers.len() == provider_record_result_count {
                    println!("found all proxies: {:#?}", found_services);
                    event_bus.sender.send(Event::PeerStoped).unwrap();
                    break; // got all we need
                }
              },
              _ => {}
            }
          },
          _ = &mut timeout_f => {
            eprintln!("find providers timeout");
            break;
          },
        }
    }

    // TODO: filter the proxy urls

    Ok(found_services)
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
    log::debug!("Local peer id: {:?}", local_peer_id);

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
    //let mut stdin = io::BufReader::new(io::stdin()).lines();

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
            log::debug!("is bootnode, addr {}", &addr);
            continue; // DO NOT ADD MYSELF
        }
        log::debug!("add boot addr {}", &addr);
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
            //line = stdin.next_line() => handle_input_line(&mut swarm.behaviour_mut().kademlia, line.expect("Stdin not to close").unwrap()),
            event = rx.recv() => {
                match event? {
                    Event::StartProviding { key } => {
                        swarm.behaviour_mut().kademlia.start_providing(Key::new(&key)).expect("Failed to start providing key");
                    },
                    Event::StopProviding { key } => {
                        swarm.behaviour_mut().kademlia.stop_providing(&Key::new(&key));
                    },
                    Event::PutRecord { key, value } => {
                        swarm.behaviour_mut().kademlia.put_record(Record {
                            key: Key::new(&key),
                            value: value,
                            publisher: None,
                            expires: None,
                        }, Quorum::One).expect("Failed to store record locally.");
                    },
                    Event::GetRecord { key } => {
                        swarm.behaviour_mut().kademlia.get_record(Key::new(&key), Quorum::One);
                    },
                    Event::GetProviders { key } => {
                        swarm.behaviour_mut().kademlia.get_providers(Key::new(&key));
                    },
                    Event::PeerStoped => {
                        break;
                    },
                    _ => {},
                }
            },
            event = swarm.select_next_some() => {
                match event {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        log::debug!("Listening on {:?}", address);
                        let mut ip: Option<String> = None;
                        let mut port: Option<u32> = None;
                        let mut is_loopback = false;
                        for addr in address.iter() {
                            match addr {
                                Protocol::Ip4(ipv4_addr) => {
                                    if ipv4_addr.is_loopback() {
                                        is_loopback = true;
                                    }
                                    ip = Some(ipv4_addr.to_string());
                                },
                                Protocol::Tcp(p) => {
                                    port = Some(p as u32);
                                }
                                _ => {},
                            }
                        }
                        if !is_loopback && ip.is_some() && port.is_some() {
                            tx.send(Event::PeerStarted { 
                                peer_id: local_peer_id.to_string(),
                                addr: ip.unwrap(),
                                port: port.unwrap() 
                            }).unwrap();
                        }
                    },
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, ..} => {
                    },
                    SwarmEvent::ConnectionClosed { peer_id, endpoint, ..} => {
                        log::debug!("ConnectionClosed, peer {:?}, endpoint {:?}", peer_id, endpoint);
                    },
                    SwarmEvent::Behaviour(message) => {
                        match message {
                            OutEvent::Identify(event) => {
                                match event {
                                    IdentifyEvent::Received {  peer_id, info } => {
                                        log::debug!("IdentifyEvent::Received, peer_id {:?}, info {:?}", peer_id, info);
                                        if info.protocols.contains(&String::from("/ipfs/kad/1.0.0")) {
                                            for addr in info.listen_addrs {
                                                swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                                            }
                                        }
                                    },
                                    IdentifyEvent::Sent {  peer_id } => {
                                        log::debug!("IdentifyEvent::Sent, peer_id {:?}", peer_id);
                                    },
                                    IdentifyEvent::Pushed {  peer_id } => {
                                        log::debug!("IdentifyEvent::Sent, peer_id {:?}", peer_id);
                                    },
                                    IdentifyEvent::Error {  peer_id, error } => {
                                        eprintln!("IdentifyEvent::Sent, peer_id {:?}, error {:?}", peer_id, error);
                                    },
                                    _ => {}
                                }
                            },
                            OutEvent::Kademlia(event) => {
                                match event {
                                    KademliaEvent::RoutingUpdated { peer, is_new_peer, addresses, .. } => {
                                        log::debug!("RoutingUpdated, peer {:?}, is_new_peer {:?}, addresses {:#?}", peer, is_new_peer, addresses);
                                    },
                                    KademliaEvent::UnroutablePeer { peer } => {
                                        log::debug!("UnroutablePeer, peer {:?}", peer);
                                    },
                                    KademliaEvent::RoutablePeer { peer, address } => {
                                        log::debug!("RoutablePeer, peer {:?}, address {:?}", peer, address);
                                    },
                                    KademliaEvent::PendingRoutablePeer { peer, address } => {
                                        log::debug!("PendingRoutablePeer, peer {:?}, address {:?}", peer, address);
                                    },
                    
                                    KademliaEvent::OutboundQueryCompleted { result, .. } => match result {
                                        QueryResult::GetProviders(Ok(ok)) => {
                                            let mut providers: Vec<String> = vec![]; 
                                            for peer in ok.providers {
                                                log::debug!(
                                                    "Peer {:?} provides key {:?}",
                                                    peer,
                                                    std::str::from_utf8(ok.key.as_ref()).unwrap()
                                                );
                                                providers.push(peer.to_string());
                                            }
                                            tx.send(Event::GetProvidersResult {
                                                key: String::from_utf8(ok.key.to_vec()).unwrap(),
                                                providers: providers,
                                            }).unwrap();
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
                                                log::debug!(
                                                    "Got record {:?} {:?}",
                                                    std::str::from_utf8(key.as_ref()).unwrap(),
                                                    value
                                                );
                                                tx.send(Event::GetRecordResult {
                                                    key: String::from_utf8(key.to_vec()).unwrap(),
                                                    value: value,
                                                }).unwrap();
                                            }
                                        }
                                        QueryResult::GetRecord(Err(err)) => {
                                            eprintln!("Failed to get record: {:?}", err);
                                        }
                                        QueryResult::PutRecord(Ok(PutRecordOk { key })) => {
                                            log::debug!(
                                                "Successfully put record {:?}",
                                                std::str::from_utf8(key.as_ref()).unwrap()
                                            );
                                            tx.send(Event::PutRecordResult { success: true }).unwrap();
                                        }
                                        QueryResult::PutRecord(Err(err)) => {
                                            eprintln!("Failed to put record: {:?}", err);
                                            tx.send(Event::PutRecordResult { success: false }).unwrap();
                                        }
                                        QueryResult::StartProviding(Ok(AddProviderOk { key })) => {
                                            log::debug!(
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
    println!("exit p2p");
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

// RUST_LOG=DEBUG BOOTNODE=true  cargo test -- --nocapture test_join_p2p
// RUST_LOG=DEBUG BOOTNODE=false cargo test -- --nocapture test_join_p2p
#[tokio::test(start_paused = true)]
async fn test_join_p2p() {
    env_logger::init();

    let key_file: String = String::from("private_UUxa.pk8");
    let bootnode_port: u32 = 53308;
    let bootnodes: Vec<String> = vec![
        String::from("/ip4/127.0.0.1/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa"),
    ];

    let key_file = match std::env::var("BOOTNODE") {
        Ok(_) => Some(key_file),
        Err(_) => None,
    };
    let event_bus = EventBus::new();
    join_p2p(key_file, bootnode_port, bootnodes.to_vec(), event_bus.clone()).await.unwrap();
}

// RUST_LOG=DEBUG cargo test -- --nocapture test_discover_p2p_services
#[tokio::test]
async fn test_discover_p2p_services() {
    env_logger::init();

    let key_file: String = String::from("private_UUxa.pk8");
    let bootnode_port: u32 = 53308;
    let bootnodes: Vec<String> = vec![
        String::from("/ip4/127.0.0.1/tcp/53308/p2p/QmVN7pykS5HgjHSGS3TSWdGqmdBkhsSj1G5XLrTconUUxa"),
    ];

    let event_bus = EventBus::new();
    let event_bus1 = event_bus.clone();
    let key_file1 = key_file.clone();
    let bootnodes1 = bootnodes.clone();
    let handle = tokio::spawn(async move {
        join_p2p(Some(key_file1), bootnode_port, bootnodes1.to_vec(), event_bus1.clone()).await.unwrap();
        println!("end join_p2p");
    });

    fn on_ready(event_bus: &EventBus, key_file: &String, port: u32, bootnodes: &Vec<String>, peer_id: String, addr: String) {
        let mut service: SSProxyService = SSProxyService::default();
        service.info.addr = addr.clone();
        service.info.port = 1080;
        service.info.password = String::from("foo!bar!");
    
        let service_key = format!("{}_{}", service.service_name(), peer_id.clone());
        event_bus.sender.send(Event::PutRecord { key: service_key, value: service.serialize() }).unwrap();
        event_bus.sender.send(Event::StartProviding { key: SSProxyService::default().service_name() }).unwrap();

        let service_name = service.service_name().clone();
        let event_bus2 = event_bus.clone();
        let key_file2 = key_file.clone();
        let bootnodes2 = bootnodes.clone();
        tokio::spawn(async move {
            let remote_proxy_services: Vec<SSProxyService> = discover_p2p_services(service_name, None, 0, bootnodes2, Duration::from_secs(60)).await.unwrap();
            println!("found proxy services: {:#?}", remote_proxy_services);
            event_bus2.sender.send(Event::PeerStoped).unwrap();

            assert!(remote_proxy_services.len() > 0);
            assert!(remote_proxy_services[0].info.addr == addr);
            assert!(remote_proxy_services[0].info.port as u32 == 1080);
        });
        println!("end on ready");
    }

    tokio::pin!(event_bus);
    loop {
        tokio::select! {
            e = event_bus.receiver.recv() => {
                match e.unwrap() {
                    Event::PeerStarted { peer_id, addr, port } => {
                        println!("peer is ready, peer_id={}, addr={}, port={}", peer_id, addr, port);
                        on_ready(&event_bus, &key_file, port, &bootnodes, peer_id, addr);
                    },
                    Event::PeerStoped => {
                        println!("peer is stoped");
                        break;
                    }
                    _ => {},
                }
            }
        }
        println!("loop")
    }
    tokio::join!(handle);
    println!("loop end")
}