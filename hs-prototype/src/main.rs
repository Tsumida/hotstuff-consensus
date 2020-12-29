//! Demo
use std::{collections::HashMap, sync::Arc, time::Duration};

use actix_web::{web, App, HttpResponse, HttpServer};
use log::{debug, error, info};
use simplelog::*;
use tokio::spawn;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::time::delay_for;

mod safety;
mod utils;
use safety::{basic::*, msg::*, Machine};

mod pacemaker;
use pacemaker::DefaultPacemaker;

/*
const PROP_COMMITTED: &str = "committed";
const PROP_LOCKED: &str = "locked";
const PROP_QUEUING: &str = "queuing";
*/

struct HotStuffProxy {
    peer_conf: HashMap<ReplicaID, String>,
    self_id: String,
    self_addr: String,
    // to hotstuff
    sender: Sender<InternalMsg>,
    // from hotstuff
    recvr: Receiver<InternalMsg>,
}

struct SharedState {
    //replica_id: ReplicaID,
    //from: ReplicaID,
    ctx: Context,
    sender: Sender<InternalMsg>,
}

impl HotStuffProxy {
    fn new(
        self_id: String,
        self_addr: String,
        peer_conf: HashMap<ReplicaID, String>,
    ) -> (Self, Sender<InternalMsg>, Receiver<InternalMsg>) {
        let (nk_sender, recvr) = channel(64);
        let (sender, nk_recvr) = channel(64);

        let nk = HotStuffProxy {
            self_id,
            self_addr,
            peer_conf,
            sender: nk_sender,
            recvr: nk_recvr,
        };

        (nk, sender, recvr)
    }

    async fn status(data: web::Data<SharedState>) -> HttpResponse {
        let (ss_sender, mut ss_recvr) = channel(1);
        let mut sender = data.sender.clone();
        let ctx = &data.ctx;
        if let Err(e) = sender.send(InternalMsg::RequestSnapshot(ss_sender)).await {
            error!("{}", e);
            return HttpResponse::Ok().json(RpcResponse::status(ctx, None));
        }

        let resp = match ss_recvr.recv().await {
            None => {
                error!("sender dropped early");
                RpcResponse::status(ctx, None)
            }
            Some((ctx, ss)) => RpcResponse::status(&ctx, Some(ss)),
        };

        HttpResponse::Ok().json(resp)
    }

    // #[post()]
    async fn recv_new_proposal(
        sd: web::Data<SharedState>,
        data: web::Json<RpcRequest>,
    ) -> HttpResponse {
        let mut sender = sd.sender.clone();
        let ctx = &data.ctx;
        // TODO: ugly
        let node = Arc::new(data.prop.as_ref().unwrap().as_ref().clone());
        let justify = Arc::new(data.qc.as_ref().unwrap().as_ref().clone());
        let (sk_sender, mut sk_recvr) = channel(1);

        if let Err(e) = sender
            .send(InternalMsg::RecvProposal(
                ctx.clone(),
                node,
                justify,
                sk_sender,
            ))
            .await
        {
            error!("{}", e);
            return HttpResponse::InternalServerError()
                .json(RpcResponse::internal_err(ctx, RpcType::NewProposal));
        }
        HttpResponse::Ok().json(match sk_recvr.recv().await {
            None => {
                info!("{} reject proposal", &sd.ctx.from);
                RpcResponse::reject(ctx)
            }
            Some((ctx, node, sign)) => {
                info!("{} accept proposal", &sd.ctx.from);
                RpcResponse::accept(&ctx, node.as_ref(), sign.as_ref())
            }
        })
    }

    async fn on_recv_new_view(
        sender: web::Data<SharedState>,
        data: web::Json<RpcRequest>,
    ) -> HttpResponse {
        match (&data.qc, &data.prop) {
            // TODO: replace OK().
            (Some(ref qc), Some(ref node)) => {
                debug!("server recv new view");
                let mut sender = sender.sender.clone();
                let _ = sender
                    .send(InternalMsg::RecvNewViewMsg(
                        data.ctx.clone(),
                        Arc::new(node.as_ref().clone()),
                        Arc::new(qc.as_ref().clone()),
                    ))
                    .await;
                HttpResponse::Ok().json(RpcResponse::ok(&data.ctx, RpcType::NewView))
            }
            _ => HttpResponse::Ok().json(RpcResponse::invalid_usage(&data.ctx, RpcType::NewView)),
        }
    }

    /// Recv new leader information about leader.
    async fn new_leader(
        sender: web::Data<SharedState>,
        data: web::Json<RpcRequest>,
    ) -> HttpResponse {
        let mut sender = sender.sender.clone();
        if let Some(leader) = &data.leader {
            sender
                .send(InternalMsg::NewLeader(data.ctx.clone(), leader.clone()))
                .await
                .unwrap();
            HttpResponse::Ok().json(RpcResponse::ok(&data.ctx, RpcType::NewLeader))
        } else {
            HttpResponse::Ok().json(RpcResponse::invalid_usage(&data.ctx, RpcType::NewLeader))
        }
    }

    async fn new_tx(
        sender: web::Data<SharedState>,
        data: web::Json<NewTransaction>,
    ) -> HttpResponse {
        let mut sender = sender.sender.clone();
        let resp = match sender.send(InternalMsg::NewTx(data.cmd.clone())).await {
            Err(e) => {
                error!("send new tx failed: {}", e);
                "internal error"
            }
            Ok(_) => "ok, tx is queuing",
        };
        HttpResponse::Ok().json(resp)
    }
}

fn default_timer(mut tick_sender: Sender<(u64, u64)>, lifetime: u64, step: u64) {
    tokio::spawn(async move {
        let step = step;
        let mut tick = 0;
        let mut deadline = tick + step;
        loop {
            delay_for(Duration::from_secs(4)).await;
            if tick >= lifetime || tick_sender.send((tick, deadline)).await.is_err() {
                break;
            }
            tick += 1;
            if tick > deadline {
                deadline += step;
            }
        }
        drop(tick_sender);
    });
}

fn main() {
    let _ = CombinedLogger::init(vec![TermLogger::new(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
    )]);

    info!("hotstuff demo");
    info!("hotstuff node ({}) Bytes", std::mem::size_of::<Machine>());

    let f = 1;
    let n = 3 * f + 1;
    let peer_config = (0..n)
        .map(|i| (format!("node-{}", i), format!("localhost:{}", 8000 + i)))
        .collect::<HashMap<String, String>>();

    let (_, pks, sk) = crate::utils::threshold_sign_kit(n, 2 * f);

    // run machines.
    let mut mcs = vec![];
    let mut handlers = vec![];
    let mut nks = vec![];
    let mut timers = vec![];
    // peer_conf dos not live enough.
    let backup = peer_config.clone();
    for (kv, sk_conf) in peer_config.into_iter().zip(sk) {
        // TODO: unbounded channel
        info!("mioa");
        let (mut stop, handler) = channel(1);
        let (tick_sender, tick_recvr) = channel(16);
        let (k, v) = kv;
        let (sign_id, sks) = sk_conf;
        let peers = backup.clone();
        let pks = pks.clone();
        let (hs_proxy, to_net, from_net) = HotStuffProxy::new(k.clone(), v, backup.clone());
        let mc = Machine::new(
            &k,
            sign_id as u32,
            pks,
            sks,
            (to_net.clone(), from_net),
            peers,
        );

        let pm = DefaultPacemaker::new(
            backup.clone(),
            tick_recvr,
            hs_proxy.sender.clone(),
            to_net.clone(),
            Some(format!("node-0")),
        );

        // timer
        timers.push(tick_sender);
        handlers.push(handler);
        mcs.push((mc, stop, pm));
        nks.push(hs_proxy);
    }

    let tokio_rt_handler = std::thread::spawn(move || {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async move {
                for (kit, tick_sender) in mcs.into_iter().zip(timers) {
                    default_timer(tick_sender, 1000, 5);
                    let (mut mc, mut stop, mut pm) = kit;
                    spawn(async move {
                        mc.run().await;
                        stop.send(()).await.unwrap();
                    });
                    spawn(async move {
                        pm.run().await;
                    });
                }
                for mut h in handlers {
                    h.recv().await.unwrap();
                }
            });
    });

    for hs_proxy in nks {
        std::thread::spawn(move || {
            actix_web::rt::Runtime::new()
                .unwrap()
                .block_on(run_server(hs_proxy))
        });
    }

    tokio_rt_handler.join().unwrap();
}

async fn broadcastor(
    peer_conf: &HashMap<ReplicaID, String>,
    resp_sender: Sender<InternalMsg>,
) -> (
    tokio::sync::broadcast::Sender<()>,
    tokio::sync::broadcast::Sender<RpcRequest>,
) {
    let (fan_out_s, _) = tokio::sync::broadcast::channel::<RpcRequest>(1);
    let (down_s, _) = tokio::sync::broadcast::channel::<()>(1);

    for (k, v) in peer_conf.iter() {
        let (_, addr) = (k.clone(), v.clone());
        let mut listener = fan_out_s.subscribe();
        let mut down = down_s.subscribe();
        let mut resp_sender = resp_sender.clone();
        actix_web::rt::spawn(async move {
            let mut exit = false;
            let url = format!("http://{}/hotstuff/new-proposal", addr);
            loop {
                tokio::select! {
                    Ok(ref prop) = listener.recv() => {
                        // debug!("posting {}", &url);
                        match actix_web::client::Client::default().post(&url).send_json(prop).await{
                            Ok(mut resp) =>{
                                let rpc_resp: RpcResponse = resp.json().await.unwrap();
                                if let Some(msg) = InternalMsg::from_rpc_response(rpc_resp){
                                    resp_sender.send(msg).await.unwrap();
                                }
                            },
                            Err(e) => {
                                error!("error {}", e);
                            }
                        }
                    },
                    Ok(_) = down.recv() =>{
                        exit = false;
                    }
                }
                if exit {
                    break;
                }
            }
        })
    }

    (down_s, fan_out_s)
}

async fn run_server(hs_proxy: HotStuffProxy) {
    let local = tokio::task::LocalSet::new();
    let sys = actix_web::rt::System::run_in_tokio("hotstuff proxy", &local);

    let sender_server = hs_proxy.sender.clone();
    info!("HotStuffProxy is listening");
    let addr = hs_proxy.self_addr.clone();
    let self_id = hs_proxy.self_id.clone();
    let (mut svr_sender, mut svr_recvr) = channel(1);

    // http server
    // TODO: tidy
    actix_web::rt::spawn(async move {
        info!("http server running");

        let server = HttpServer::new(move || {
            App::new()
                .data(SharedState {
                    // TODO: update ctx
                    ctx: Context {
                        from: self_id.clone(),
                        view: 0,
                    },
                    sender: sender_server.clone(),
                })
                .service(
                    web::scope("/hotstuff")
                        .route("status", web::get().to(HotStuffProxy::status))
                        // new proposal from leader
                        .route(
                            "new-proposal",
                            web::post().to(HotStuffProxy::recv_new_proposal),
                        )
                        // recv new view from other replicas.
                        .route("new-view", web::post().to(HotStuffProxy::on_recv_new_view))
                        // who's new leader
                        .route("new-leader", web::post().to(HotStuffProxy::new_leader))
                        // tx from client to commit
                        .route("new-tx", web::post().to(HotStuffProxy::new_tx)),
                )
        })
        .bind(addr)
        .unwrap()
        .run();

        svr_sender.send(server).await.unwrap();
    });

    // TODO: tidy
    // proxy loop
    actix_web::rt::spawn(async move {
        let svr = svr_recvr.recv().await.unwrap();
        let mut hs_proxy = hs_proxy;
        let mut quit = false;
        // let mut sender_client = sender_client;
        let (fan_out_handler, fan_out) =
            broadcastor(&hs_proxy.peer_conf, hs_proxy.sender.clone()).await;

        loop {
            tokio::select! {
                Some(msg) = hs_proxy.recvr.recv() => {
                    match msg{
                        InternalMsg::Propose(ctx, node, qc) => {
                            info!("proposing");
                            fan_out.send(
                                RpcRequest{
                                    ctx,
                                    msg_type: RpcType::NewProposal,
                                    leader: None,
                                    prop: Some(Box::new(node.as_ref().clone())),
                                    qc: Some(Box::new(qc.as_ref().clone())),
                                }
                            ).unwrap();
                        },
                        InternalMsg::Down => {
                            quit = true;
                            let _ = fan_out_handler.send(());
                        }
                        _ => error!("invalid msg"),
                    }
                },
            }
            if quit {
                break;
            }
        }
        svr.stop(false).await;
    });
    sys.await.unwrap();
}
