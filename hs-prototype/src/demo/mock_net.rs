use threshold_crypto::{
    SecretKeySet
};

use rand::thread_rng;
use std::collections::HashMap;
use std::sync::mpsc::{
    channel,
    Sender,
    Receiver,
};

use crate::{
    network::Network,  
    msg::*,
};

pub struct MockNetworkComponent{
    pub send_ch: Sender<(usize, Msg<Vec<u8>>)>,
    pub recv_ch: Receiver<Msg<Vec<u8>>>,
}


impl Network<Msg<Vec<u8>>> for MockNetworkComponent{
    fn send(&mut self, target: usize, msg: Msg<Vec<u8>>) -> (){
        self.send_ch.send((target, msg)).unwrap();
    }

    fn recv(&mut self) -> Msg<Vec<u8>>{
        self.recv_ch.recv().unwrap()
    }
}

pub struct MockNetwork{
    pub out: HashMap<usize, Sender<Msg<Vec<u8>>>>,
    pub mailbox: Receiver<(usize, Msg<Vec<u8>>)>,
}