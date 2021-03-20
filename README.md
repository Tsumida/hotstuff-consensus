# hotstuff-consensus
## Note
hotstuff-consensus is under development. Since it's buggy, more tests and refactoring and needed. 
## Arch
![arch](https://github.com/Tsumida/hotstuff-consensus/blob/dev/design/arch.png)

- HotStuff Proxy: Network event handler and deliverer. Currently the proxy use `tarpc` to send messages. 
- Machine: Implemetation of event-driven hotstuff algorithm. 
- Pacemaker: Maintains liveness. Pacemaker consists of 3 components:
  - Timer: Setting and cancelling view timer. 
  - RoudRobin Elector: mapping view to leader. 
  - Branch synchronizer: collecting all missing proposals. In current implemetation it works when it receive a proposal with an unknown proposal.justify.node and then request leader deliver all missing blocks.
- Signaturer: Verifying, signing and combining partial signatures.
- HotStuff Storage: Shared in-memory storage with a persistent backend(based on MySQL, currently).  Pacemaker and Machine access HotStuff Storage by`LivenessStorage` and `SafetyStorage` respectively. 
