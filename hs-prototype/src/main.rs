//!
//! Event-driven HotStuff prototype
//! BigEndian

use std::hash::Hash; 
use sha2::{Digest, Sha256}; 

use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian, LittleEndian};

type PK = threshold_crypto::PublicKey;
type SK = threshold_crypto::SecretKeyShare; 
type Sign = threshold_crypto::SignatureShare; 
type CombinedSign = threshold_crypto::Signature; 
type Cmd = String; 

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeHash([u8; 32]); 

impl NodeHash{
    fn genesis() -> NodeHash{
        NodeHash([0xAB; 32])
    }

    #[inline]
    fn byte_len() -> usize{
        256
    }
}

enum MsgType{ 
    // leader -> replcia
    Generic, 
    // replica -> leader
    GenericReply, 
    // replica -> leader
    NewView, 
}

struct Msg{
    msg_type: MsgType, 
    view: u64, 
    node: TreeNode, 
    justify: GenericQC, 
}

struct GenericQC{
    msg_type: MsgType, 
    view: u64, 
    node: TreeNode, 
    combined_sign: CombinedSign, 
}

#[derive(Debug, Clone)]
struct TreeNode{
    height: u64, 
    // view: u64, 
    cmds: Vec<Cmd>, 
    // meta
    parent: NodeHash, 
    // for safety
    // justify: GenericQCHash, 
}

impl TreeNode{
    fn genesis() -> TreeNode{
        TreeNode{
            height: 0,
            cmds: vec![],
            parent: NodeHash::genesis(), 
        }
    }

    fn hash(node: &TreeNode) -> NodeHash{
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

    fn new(cmds: impl IntoIterator<Item=Cmd>, height: u64, parent: NodeHash) -> TreeNode{
        TreeNode{
            cmds: cmds.into_iter().collect::<Vec<Cmd>>(), 
            height, 
            parent,
        }
    }
}

#[cfg(test)]
mod basic{
    use super::*;
    use std::collections::HashMap; 

    #[test]
    fn test_cmd_to_node(){
        let cmd = "Hello, world!".to_string();
        let genesis = TreeNode::genesis();
        let h = TreeNode::hash(&genesis);
        let first_node = TreeNode::new(
            vec!["miao".to_string(), "zhizhi".to_string(), "guagua".to_string()],
            1,
            h,
        );
    }

    #[test]
    fn test_branch(){
        let mut tree = HashMap::new(); 
        let mut height = 0;

        let genesis = TreeNode::genesis();
        let root = genesis.clone();
        let gh = TreeNode::hash(&genesis); 
        let mut parent = gh.clone(); 
        tree.insert(gh, genesis);

        let mut cmds = vec![
            vec!["hello".to_string(), "world".to_string()], 
            vec!["good to see you".to_string()], 
            vec!["miaomiao?".to_string(), "zhizhi!".to_string()], 
        ];

        for vc in cmds.clone(){
            height += 1;
            let node = TreeNode::new(
                vc.into_iter(), 
                height, 
                parent, 
            );
            let h = TreeNode::hash(&node);
            parent = h.clone(); 
            tree.insert(h, node);
        }
        // from leaf to root 
        while let Some((_, v)) = tree.get_key_value(&parent){
            let v: &TreeNode = v;
            assert!(
                v.height == height &&
                &v.cmds == cmds.get(height as usize - 1).unwrap()
            );
            parent = v.parent.clone(); 
            height -= 1;
            if height == 0 {
                break
            }
        }
    }

}


fn main() {
    
}