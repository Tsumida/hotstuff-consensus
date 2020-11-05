
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
    // pub const fn byte_len() -> usize{ 256 }
}


impl std::convert::AsRef<[u8]> for NodeHash{
    fn as_ref(&self) -> &[u8] {
        return &self.0
    }
}

pub fn sign(sk:&SK, node: &TreeNode, view: u64) -> Sign{
    let mut buf: Vec<u8> = Vec::with_capacity(264);  // 256 + 8
    buf.extend(TreeNode::hash(&node).0.iter()); 
    buf.extend(view.to_be_bytes().iter());
    sk.sign(&buf)
}

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

    // pub const fn byte_len() -> usize {256}
}


#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenericQC{
    pub view: u64, 
    pub node: TreeNode, 
    // None for bootstrapping. 
    pub combined_sign: Option<CombinedSign>, 
}

impl GenericQC{
    pub fn new(view:u64, node: TreeNode, combined_sign: CombinedSign) -> Self{
        GenericQC{
            view, node, 
            combined_sign: Some(combined_sign), 
        }
    }

    pub fn genesis(view: ViewNumber, node: &TreeNode) -> Self{
        GenericQC{
            view, 
            node: node.clone(), 
            combined_sign: None, 
        }
    }

    /*
    pub fn is_bootstrapping_qc(&self) -> bool{
        self.combined_sign.is_none()
    }
    */

    pub fn hash(&self) -> QCHash{
        let mut res = [0u8; 32]; 
        if let Some(ref v) = self.combined_sign{
            res.copy_from_slice(&v.to_bytes()[32..64]);
        }
        // None -> [0u8; 32]
        QCHash(res)
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

    /*
    #[inline]
    pub fn new(cmds: impl IntoIterator<Item=Cmd>, height: u64, parent: NodeHash, justify: QCHash) -> TreeNode{
        TreeNode{
            cmds: cmds.into_iter().collect::<Vec<Cmd>>(), 
            height, 
            parent,
            justify, 
        }
    }*/

    pub fn node_and_hash(cmds: impl IntoIterator<Item=Cmd>, height: u64, parent: NodeHash, justify: QCHash) -> (Box<TreeNode>, Box<NodeHash>){
        let node = Box::new(TreeNode{
            cmds: cmds.into_iter().collect::<Vec<Cmd>>(), 
            height, 
            parent,
            justify, 
        });

        let hash = Box::new(TreeNode::hash(&node));

        (node, hash)
    }
    
}


#[test]
fn test_branch(){
    use std::collections::HashMap; 

    // TreeNode forms a branch.
    let mut tree = HashMap::new(); 
    let mut height = 0;

    let genesis = TreeNode::genesis();
    let end = NodeHash::genesis();
    let gh = TreeNode::hash(&genesis); 
    let mut parent = gh.clone(); 
    tree.insert(gh, genesis);
    let qc_hash = QCHash::genesis();

    let cmds = vec![
        vec!["hello".to_string(), "world".to_string()], 
        vec!["good to see you".to_string()], 
        vec!["miaomiao?".to_string(), "zhizhi!".to_string()], 
    ];

    for vc in cmds.clone(){
        height += 1;
        let (node, h) = TreeNode::node_and_hash(
            vc.into_iter(), 
            height, 
            parent, 
            qc_hash.clone(), 
        );
        parent = h.as_ref().clone(); 
        tree.insert(*h, *node);
    }
    // from leaf to root 
    // Should record leaf or its hash.
    while let Some((_, v)) = tree.get_key_value(&parent){
        let v: &TreeNode = v;
        assert!(v.height == height);
        if height > 0{
            assert!(&v.cmds == cmds.get(height as usize - 1).unwrap());
            height -= 1;
        }
        parent = v.parent.clone(); 
        if parent == end{
            break;
        }
    }
}
 

#[test]
fn test_combined_sign(){

    let f = 1; 
    let n = 3 * f + 1;
    let (_, pks, vec_sk) = crate::utils::threshold_sign_kit(n, 2*f);
    let msg = "Hello, world!"; 

    let signs = vec_sk.iter()
        .map(|(_, s)| s.sign(msg))
        .collect::<Vec<Sign>>(); 

    // verify all partial signature. 
    assert!(
        signs.iter()
        .enumerate()
        .map(|(i, sk)| pks.public_key_share(i).verify(sk, msg))
        .fold(true, |s, d| s & d)
    );

    // combined_sig
    // it needs (usize, &Sign)... - - 
    let tmp = signs.iter()
        .enumerate()
        .map(|(i, s)| (i, s))
        .collect::<Vec<_>>(); 
    let combined_sig = pks.combine_signatures(tmp).expect("signs mismatch");

    assert!(
        pks.public_key().verify(&combined_sig, msg)
    );
}