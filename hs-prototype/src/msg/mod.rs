use serde::{
    Serialize, 
    Deserialize,
};

pub type Signature = threshold_crypto::SignatureShare;
pub type PrivateKey = threshold_crypto::SecretKeyShare;
pub type PublicKey = threshold_crypto::PublicKeySet;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, )]
pub enum MsgType{
    Raw, 
    NewView,
    Prepare,
    PreCommit,
    Commit,
    Decide,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
pub struct Msg<T>{
    pub msg_type: MsgType, 
    pub from: usize, 
    pub cont: T,
}