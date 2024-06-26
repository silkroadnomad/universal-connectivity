use anyhow::{Context, Result};
use clap::Parser;
use futures::future::{select, Either};
use futures::StreamExt;
// use futures::stream::StreamExt;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::{
    core::muxing::StreamMuxerBox,
    yamux, noise,
    tcp,
    ping,
    dcutr,
    dns, gossipsub, identify, identity,
    memory_connection_limits,
    multiaddr::{Multiaddr, Protocol},
    quic, relay,
    swarm::{NetworkBehaviour, Swarm, SwarmEvent},
    PeerId, StreamProtocol, SwarmBuilder, Transport
};
use libp2p_webrtc as webrtc;
// use libp2p::Transport;
use libp2p_webrtc::tokio::Certificate;
use log::{debug, error, info, warn};
use prost::Message;
use std::net::IpAddr;
use std::path::Path;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};
use tokio::fs;

include!(concat!(env!("OUT_DIR"), "/decontact.rs"));

const TICK_INTERVAL: Duration = Duration::from_secs(15);
const PORT_TCP: u16 = 1234;
const PORT_WEBRTC: u16 = 9090;
const PORT_QUIC: u16 = 9091;
const LOCAL_KEY_PATH: &str = "./local_key";
const LOCAL_CERT_PATH: &str = "./cert.pem";
const GOSSIPSUB_PEER_DISCOVERY: &str = "dcontact._peer-discovery._p2p._pubsub";
const DCONTACT_TOPIC: &str = "/dContact/3/message/proto";

#[derive(Debug, Parser)]
#[clap(name = "universal connectivity rust peer")]
struct Opt {
    /// Address to listen on.
    #[clap(long, default_value = "0.0.0.0")]
    listen_address: IpAddr,

    /// If known, the external address of this node. Will be used to correctly advertise our external address across all transports.
    #[clap(long, env)]
    external_address: Option<IpAddr>,

    /// Gossipsub peer discovery topic.
    #[clap(long, default_value = GOSSIPSUB_PEER_DISCOVERY)]
    gossipsub_peer_discovery: String,

    /// Gossipsub peer discovery topic.
    #[clap(long, default_value = DCONTACT_TOPIC)]
    dcontact_topic: String,

    #[clap(
        long,
        default_value = "/dns4/ipfs.le-space.de/tcp/1235/p2p/12D3KooWAJjbRkp8FPF5MKgMU53aUTxWkqvDrs4zc1VMbwRwfsbE"
    )]
    connect: Vec<Multiaddr>
}

/// An example WebRTC peer that will accept connections
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let opt = Opt::parse();
    let local_key = read_or_create_identity(Path::new(LOCAL_KEY_PATH))
        .await
        .context("Failed to read identity")?;
    let webrtc_cert = read_or_create_certificate(Path::new(LOCAL_CERT_PATH))
        .await
        .context("Failed to read certificate")?;

    let mut swarm = create_swarm(local_key, webrtc_cert, &opt)?;

    let address_tcp = Multiaddr::from(opt.listen_address)
        .with(Protocol::Tcp(PORT_TCP));

    let address_webrtc = Multiaddr::from(opt.listen_address)
         .with(Protocol::Udp(PORT_WEBRTC))
         .with(Protocol::WebRTCDirect);

    let address_quic = Multiaddr::from(opt.listen_address)
        .with(Protocol::Udp(PORT_QUIC))
        .with(Protocol::QuicV1);

    swarm
        .listen_on(address_tcp.clone())
        .expect("listen on tcp");
    swarm
        .listen_on(address_webrtc.clone())
        .expect("listen on webrtc");
    swarm
        .listen_on(address_quic.clone())
        .expect("listen on quic");

    for addr in opt.connect {
        if let Err(e) = swarm.dial(addr.clone()) {
            debug!("Failed to dial {addr}: {e}");
        }
    }

    let peer_discovery = gossipsub::IdentTopic::new(GOSSIPSUB_PEER_DISCOVERY).hash();
    let dcontact_topic = gossipsub::IdentTopic::new(DCONTACT_TOPIC).hash();

    let mut tick = futures_timer::Delay::new(TICK_INTERVAL);

    loop {
        match select(swarm.next(), &mut tick).await {
            Either::Left((event, _)) => match event.unwrap() {
                SwarmEvent::NewListenAddr { address, .. } => {
                    if let Some(external_ip) = opt.external_address {
                        let external_address = address
                            .replace(0, |_| Some(external_ip.into()))
                            .expect("address.len > 1 and we always return `Some`");

                        swarm.add_external_address(external_address);
                    }

                    let p2p_address = address.with(Protocol::P2p(*swarm.local_peer_id()));
                    info!("Listening on {p2p_address}");
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    info!("Connected to {peer_id}");
                }
                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    warn!("Failed to dial {peer_id:?}: {error}");
                }
                SwarmEvent::IncomingConnectionError { error, .. } => {
                    warn!("{:#}", anyhow::Error::from(error))
                }
                SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    warn!("Connection to {peer_id} closed: {cause:?}");
//                     swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
//                     info!("Removed {peer_id} from the routing table (if it was in there).");
                }
                SwarmEvent::Behaviour(BehaviourEvent::Relay(e)) => {
                    debug!("{:?}", e);
                }
                SwarmEvent::Behaviour(BehaviourEvent::Dcutr(e)) => {
                    info!("Connected to {:?}", e);
                }

                // Ping event
          /*      SwarmEvent::Behaviour(BehaviourEvent::Ping(ping::Event {
                    peer,
                    result: Ok(rtt),
                    ..
                })) => {
                     debug!("🏓 Ping {peer} in ");
                    // debug!("🏓 Ping {peer} in {rtt:?}");

                    // send msg
                    self.event_sender
                        .send(NetworkEvent::Pong {
                            peer: peer.to_string(),
                            rtt: rtt.as_millis() as u64,
                        })
                        .await
                        .expect("Event receiver not to be dropped.");
                } */

                SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(
                    libp2p::gossipsub::Event::Message {
                        message_id: _,
                        propagation_source: _,
                        message,
                    },
                )) => {
                         // subscribe to this topic so we can act as super peer to browsers
                        let newTopic = gossipsub::IdentTopic::new(message.topic.to_string());
                        //swarm.behaviour_mut().gossipsub.subscribe(&newTopic)?;
                        if let Err(err) =
                            swarm.behaviour_mut().gossipsub.subscribe(&newTopic)
                        {
                            error!("Failed to subscribe to topic: {err}");
                        }
                       info!(" subscribe to topic:  to {:?}", message.topic);
//                     if message.topic == peer_discovery {
//                         let peer = Peer::decode(&*message.data).unwrap();
//                         //info!("Received peer from {:?}", peer.addrs);
//                         for addr in &peer.addrs {
//                             if let Ok(multiaddr) = Multiaddr::try_from(addr.clone()) {
//                                 info!("Received address: {:?}", multiaddr.to_string());
//
//                                 if let Err(err) = swarm.behaviour_mut().gossipsub.publish(
//                                                          gossipsub::IdentTopic::new(GOSSIPSUB_PEER_DISCOVERY),
//                                                          &*message.data,)
//                                 {error!("Failed to publish peer: {err}")}
//                             } else {
//                                         error!("Failed to parse multiaddress");
//                             }
//                         }
//                     }

//                     if message.topic == dcontact_topic {
//                         let peer = Peer::decode(&*message.data).unwrap();
//                         //info!("Received peer from {:?}", peer.addrs);
//                         for addr in &peer.addrs {
//                             if let Ok(multiaddr) = Multiaddr::try_from(addr.clone()) {
//                                 info!("Received address: {:?}", multiaddr.to_string());
//
//                                 if let Err(err) = swarm.behaviour_mut().gossipsub.publish(
//                                                          gossipsub::IdentTopic::new(DCONTACT_TOPIC),
//                                                          &*message.data,)
//                                 {error!("Failed to publish peer: {err}")}
//                             } else {
//                                 error!("Failed to parse multiaddress");
//                             }
//                         }
//
//                         continue;
//                     }

//                     error!("Unexpected gossipsub topic hash: {:?}", message.topic);
                }
                SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(
                    libp2p::gossipsub::Event::Subscribed { peer_id, topic },
                )) => {
                        debug!("{peer_id} subscribed to {topic}");

                         // Indiscriminately add the peer to the routing table
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);

                }

                SwarmEvent::Behaviour(BehaviourEvent::Identify(e)) => {
                    info!("BehaviourEvent::Identify {:?}", e);

                    if let identify::Event::Error { peer_id, error } = e {
                        match error {
                            libp2p::swarm::StreamUpgradeError::Timeout => {
                                info!("Removed {peer_id} from the routing table (if it was in there).");
                            }
                            _ => {
                                debug!("{error}");
                            }
                        }
                    } else if let identify::Event::Received {
                        peer_id,
                        info:
                            identify::Info {
                                listen_addrs,
                                protocols,
                                observed_addr,
                                ..
                            },
                    } = e
                    {
                        debug!("identify::Event::Received observed_addr: {}", observed_addr);
                        swarm.add_external_address(observed_addr);
                    }
                },
                _ => {},
            },
            Either::Right(_) => {
                tick = futures_timer::Delay::new(TICK_INTERVAL);

                debug!(
                    "external addrs: {:?}",
                    swarm.external_addresses().collect::<Vec<&Multiaddr>>()
                );
            }
        }
    }
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    ping: ping::Behaviour,
    dcutr: dcutr::Behaviour,
    gossipsub: gossipsub::Behaviour,
    identify: identify::Behaviour,
    relay: relay::Behaviour,
    //relay: relay::Behaviour::new(key.public().to_peer_id(), Default::default()),
//     request_response: request_response::Behaviour<FileExchangeCodec>,
    connection_limits: memory_connection_limits::Behaviour,
}

fn create_swarm(
    local_key: identity::Keypair,
    certificate: Certificate,
    opt:&Opt
) -> Result<Swarm<Behaviour>> {
    let local_peer_id = PeerId::from(local_key.public());
    debug!("Local peer id: {local_peer_id}");

    // To content-address message, we can take the hash of message and use it as an ID.
    let message_id_fn = |message: &gossipsub::Message| {
        let mut s = DefaultHasher::new();
        message.data.hash(&mut s);
        gossipsub::MessageId::from(s.finish().to_string())
    };

    // Set a custom gossipsub configuration
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Permissive) // This sets the kind of message validation. The default is Strict (enforce message signing)
        .message_id_fn(message_id_fn) // content-address messages. No two messages of the same content will be propagated.
        .mesh_outbound_min(1)
        .mesh_n_low(1)
        .flood_publish(true)
        .build()
        .expect("Valid config");

    // build a gossipsub network behaviour
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(local_key.clone()),
        gossipsub_config,
    )
    .expect("Correct configuration");

    // Create/subscribe Gossipsub topics
    gossipsub.subscribe(&gossipsub::IdentTopic::new(&opt.gossipsub_peer_discovery))?;

//     let transport = {
//         let webrtc = webrtc::tokio::Transport::new(local_key.clone(), certificate);
//         let quic = quic::tokio::Transport::new(quic::Config::new(&local_key));
//
//         let mapped = webrtc.or_transport(quic).map(|fut, _| match fut {
//             Either::Right((local_peer_id, conn)) => (local_peer_id, StreamMuxerBox::new(conn)),
//             Either::Left((local_peer_id, conn)) => (local_peer_id, StreamMuxerBox::new(conn)),
//         });
//
//         dns::TokioDnsConfig::system(mapped)?.boxed()
//     };

    let identify_config = identify::Behaviour::new(
        identify::Config::new("/ipfs/0.1.0".into(), local_key.public())
            .with_interval(Duration::from_secs(60)), // do this so we can get timeouts for dropped WebRTC connections
    );

    let behaviour = Behaviour {
        ping: ping::Behaviour::new(ping::Config::new()),
        dcutr: dcutr::Behaviour::new(local_key.public().to_peer_id()),
        gossipsub,
        identify: identify_config,
        relay: relay::Behaviour::new(
            local_peer_id,
            relay::Config {
                max_reservations: usize::MAX,
                max_reservations_per_peer: 100,
                reservation_rate_limiters: Vec::default(),
                circuit_src_rate_limiters: Vec::default(),
                max_circuits: usize::MAX,
                max_circuits_per_peer: 100,
                ..Default::default()
            },
        ),
        connection_limits: memory_connection_limits::Behaviour::with_max_percentage(0.9),
    };

    let swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_other_transport(|id_keys| {
            Ok(webrtc::tokio::Transport::new(
                id_keys.clone(),
               certificate,
            )
            .map(|(peer_id, conn), _| (peer_id, StreamMuxerBox::new(conn))))
        })?
        .with_behaviour(|key| behaviour)?
        .build();

    Ok(swarm)
}

async fn read_or_create_certificate(path: &Path) -> Result<Certificate> {
    if path.exists() {
        let pem = fs::read_to_string(&path).await?;

        info!("Using existing certificate from {}", path.display());

        return Ok(Certificate::from_pem(&pem)?);
    }

    let cert = Certificate::generate(&mut rand::thread_rng())?;
    fs::write(&path, &cert.serialize_pem().as_bytes()).await?;

    info!(
        "Generated new certificate and wrote it to {}",
        path.display()
    );

    Ok(cert)
}

async fn read_or_create_identity(path: &Path) -> Result<identity::Keypair> {
    if path.exists() {
        let bytes = fs::read(&path).await?;

        info!("Using existing identity from {}", path.display());

        return Ok(identity::Keypair::from_protobuf_encoding(&bytes)?); // This only works for ed25519 but that is what we are using.
    }

    let identity = identity::Keypair::generate_ed25519();

    fs::write(&path, &identity.to_protobuf_encoding()?).await?;

    info!("Generated new identity and wrote it to {}", path.display());

    Ok(identity)
}