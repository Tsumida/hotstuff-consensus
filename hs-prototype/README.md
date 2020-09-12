# hs-prototype

## Demo
function `demo()` presents communication between nodes using threshold signature.
```
// for n = 4
mocknetwork: msg 2 -> 0
mocknetwork: msg 2 -> 1
mocknetwork: msg 2 -> 3
mocknetwork: msg 3 -> 2
mocknetwork: msg 1 -> 2
mocknetwork: msg 0 -> 2
ok, leader-2 generates final signature: Signature(10fb..06be)
mocknetwork: msg 2 -> 0
mocknetwork: msg 2 -> 1
mocknetwork: msg 2 -> 3
follower-0 verified proposal: proposal from 2
follower-1 verified proposal: proposal from 2
follower-3 verified proposal: proposal from 2
```