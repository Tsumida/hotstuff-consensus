pub enum NetworkResponse<T>{
    Timeout, 
    Reply(T),
}

pub trait Network<T>{
    // Send a signature
    fn send(&mut self, target: usize, msg: T) -> ();

    // Blocking until recv signature from other node.
    // timeout_ms
    fn recv(&mut self) -> T ;

    //fn wait_qc(&mut self, threshold:usize, timeout: u64) -> NetworkResult<QC>; 
}