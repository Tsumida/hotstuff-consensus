
use std::hash::Hash; 
use sha2::{Digest, Sha256}; 

use serde::{
    Serialize, 
    Deserialize, 
};


pub type PK = threshold_crypto::PublicKeySet;
pub type SK = threshold_crypto::SecretKeyShare; 
pub type Sign = threshold_crypto::SignatureShare; 
pub type CombinedSign = threshold_crypto::Signature; 
pub type ReplicaID = String;  
pub type ViewNumber = u64; 
pub type Cmd = String; 
pub type SignID = u32; 

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SignKit{
    pub sign: Sign, 
    pub sign_id: SignID, 
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Role{
    Leader, 
    Follower, 
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeHash(pub [u8; 32]); 

impl NodeHash{
    pub fn genesis() -> NodeHash{
        NodeHash([0xAB; 32])
    }
}


impl std::convert::AsRef<[u8]> for NodeHash{
    fn as_ref(&self) -> &[u8] {
        return &self.0
    }
}

/*
pub fn sign(sk:&SK, node: &TreeNode, view: u64) -> Sign{
    let mut buf: Vec<u8> = Vec::with_capacity(264);  // 256 + 8
    buf.extend(TreeNode::hash(&node).0.iter()); 
    buf.extend(view.to_be_bytes().iter());
    sk.sign(&buf)
}*/

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QCHash(pub [u8; 32]);

impl std::convert::AsRef<[u8]> for QCHash{
    fn as_ref(&self) -> &[u8] {
        return &self.0
    }
}

impl QCHash{
    pub fn genesis() -> QCHash{
        QCHash([0; 32])
    }
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenericQC{
    // view equals node.height 
    pub view: u64, 
    pub node: NodeHash, 
    // None for bootstrapping. 
    pub combined_sign: Option<CombinedSign>, 
}

impl GenericQC{
    pub fn new(view:u64, node: &NodeHash, combined_sign: &CombinedSign) -> Self{
        GenericQC{
            view, 
            node: node.clone(), 
            combined_sign: Some(combined_sign.clone()), 
        }
    }

    pub fn genesis(view: ViewNumber, node: &TreeNode) -> Self{
        GenericQC{
            view, 
            node: TreeNode::hash(node), 
            combined_sign: None, 
        }
    }

    pub fn hash(&self) -> QCHash{
        let mut res = [0u8; 32]; 
        if let Some(ref v) = self.combined_sign{
            res.copy_from_slice(&v.to_bytes()[32..64]);
        }
        // None -> [0u8; 32]
        QCHash(res)
    }   
    
    pub fn to_be_bytes(&self) -> Vec<u8>{
        let size = 8 
        + std::mem::size_of::<NodeHash>() 
        + if self.combined_sign.is_some(){
            96
        }else{ 0 }; 

        let mut buf = Vec::with_capacity(size); 
        buf.extend_from_slice(&self.view.to_be_bytes()); 
        buf.extend_from_slice(&self.node.as_ref());
        if self.combined_sign.is_some(){
            buf.extend_from_slice(&self.combined_sign.as_ref().unwrap().to_bytes());
        }   

        buf
    }
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeNode{
    pub height: u64,  // height === viewNumber
    pub cmds: Vec<Cmd>, 
    pub parent: NodeHash, 
    pub justify: QCHash, 
}

impl TreeNode{
    pub fn genesis() -> TreeNode{
        TreeNode{
            height: 0,
            cmds: vec![],
            parent: NodeHash::genesis(), 
            justify: QCHash::genesis(), 
        }
    }

    /// Hash using sha256. 
    pub fn hash(node: &TreeNode) -> NodeHash{
        let mut h = Sha256::default();
        for s in &node.cmds{
            h.update(s);
        }
        h.update(node.height.to_be_bytes());
        h.update(node.parent.0);

        let mut res = [0; 32];
        res.copy_from_slice(&h.finalize());
        NodeHash(res)
    }

    pub fn node_and_hash<'a>(cmds: impl IntoIterator<Item=&'a Cmd>, height: u64, parent: &NodeHash, justify: &QCHash) -> (Box<TreeNode>, Box<NodeHash>){
        let node = Box::new(TreeNode{
            cmds: cmds.into_iter().cloned().collect::<Vec<Cmd>>(), 
            height, 
            parent: parent.clone(),
            justify: justify.clone(), 
        });

        let hash = Box::new(TreeNode::hash(&node));

        (node, hash)
    }

    pub fn to_be_bytes(&self) -> Vec<u8>{
        let mut size = std::mem::size_of_val(&self.height);
        size += self.justify.as_ref().len(); 
        size += self.parent.as_ref().len(); 
        size += self.cmds.iter().fold(0, |n, s| n + s.len()); 

        let mut buf = Vec::with_capacity(size); 
        buf.extend_from_slice(&self.height.to_be_bytes());
        buf.extend_from_slice(self.justify.as_ref());
        buf.extend_from_slice(self.parent.as_ref());
        for c in &self.cmds{
            buf.extend_from_slice(c.as_bytes()); 
        }
        buf
    }
}