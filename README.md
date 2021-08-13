# hotstuff-consensus
## Intro
hotstuff-consensus is a Rust implementation of https://dl.acm.org/doi/10.1145/3293611.3331591. 
This project is under development. Since it's buggy, more tests and refactoring are needed. 

## Arch
![arch](https://github.com/Tsumida/hotstuff-consensus/blob/dev/design/arch.png)

- HotStuff Proxy: Network event handler and deliverer. Currently the proxy use `tarpc` to implement reliable event singlecast and broadcast. 
- Machine: Implemetation of event-driven hotstuff algorithm. 
- Pacemaker: Maintains liveness. Pacemaker consists of 3 components:
  - Timer: Setting and cancelling view timer. 
  - RoudRobin Elector: mapping view to leader. 
  - Branch synchronizer: collecting all missing proposals. In current implemetation it works when it receive a proposal with an unknown proposal.justify.node and then require leader deliver all missing blocks.
- Signaturer: Verifying, signing and combining partial signatures. This module is stateless and used by Machine, Pacemaker and HotStuffStorage. 
- HotStuff Storage: Shared in-memory storage with a persistent backend(currently based on MySQL).  Pacemaker and Machine access HotStuff Storage by`LivenessStorage` and `SafetyStorage` respectively. 
