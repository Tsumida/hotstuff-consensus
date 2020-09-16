pub enum TickBase{
    Second,
    MillSec,
    Minute,
}

pub trait DelayQueue<E>{

    fn set_tick_base(&mut self, tb: TickBase);

    fn push(&mut self, event: E, delay: u64) -> &mut Self;

    fn now(&self) -> u64;

    // pop all event
    fn trigger(&mut self) -> Option<Vec<(E, u64)>>;

    // advance timer.
    fn tick(&mut self);

    fn advance(&mut self, step: u64);
}

use std::collections::BinaryHeap;
use std::cmp::{
    PartialEq, PartialOrd, Ord, Eq,
    Reverse,
    Ordering,
};


#[derive(Eq)]
pub struct EventWithTick<E:Eq>{
    event: E,
    triggered_time: u64,
}

// Compare triggeered
impl<E:Eq> PartialEq for EventWithTick<E>{
    fn eq(&self, other: &Self) -> bool{
        self.triggered_time == other.triggered_time
    }
}

impl<E:Eq> PartialOrd for EventWithTick<E>{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering>{
        self.triggered_time
            .partial_cmp(&other.triggered_time)
    }
}


impl<E:Eq> Ord for EventWithTick<E>{
    fn cmp(&self, other: &Self) -> Ordering{
        self.triggered_time.cmp(&other.triggered_time)
    }
}

pub struct TickQueue<E:Eq>{
    now: u64,
    tick_base: TickBase,
    queue: BinaryHeap<Reverse<EventWithTick<E>>>,
}

impl<E:Eq> DelayQueue<E> for TickQueue<E>{
    fn set_tick_base(&mut self, tick_base: TickBase){
        self.tick_base = tick_base;
    }

    fn push(&mut self, event: E, delay: u64) -> &mut Self{
        self.queue.push(
            Reverse(EventWithTick{
                event: event,
                triggered_time: self.now() + delay,
            })
        );
        self
    }

    fn now(&self) -> u64{
        self.now
    }

    // pop all event
    fn trigger(&mut self) -> Option<Vec<(E, u64)>>{
        let mut events = Vec::new();
        loop{
            let flag = match self.queue.peek(){
                None => false,
                Some(Reverse(e)) => e.triggered_time <= self.now(),
            };
            if !flag{
                break
            }
            let Reverse(e) = self.queue.pop().unwrap();
            events.push((e.event, e.triggered_time));
        }
        if events.len() > 0{
            Some(events)
        }else{
            None
        }
    }

    // advance timer.
    #[inline]
    fn tick(&mut self){
        self.now += 1;
    }

    #[inline]
    fn advance(&mut self, step: u64){
        self.now += step;
    }
}

impl<E:Eq> TickQueue<E>{
    fn event_num(&self) -> usize{
        self.queue.len()
    }
}

#[test]
fn test_tick_queue() {
    let mut tq = TickQueue{
        now: 10u64,
        tick_base: TickBase::Second,
        queue: BinaryHeap::new(),
    };
    tq.push("world!", 10)
        .push("Hello, ", 5);

    assert_eq!(2, tq.event_num());

    tq.advance(5);
    assert_eq!(15, tq.now());
    assert_eq!(
        tq.trigger().unwrap(),
        vec![("Hello, ", 15)],
    );

    tq.advance(5);
    assert_eq!(20, tq.now());
    assert_eq!(
        tq.trigger().unwrap(),
        vec![("world!", 20)],
    );
}

#[test]
fn test_advance() {
    let mut tq = TickQueue{
        now: 0u64,
        tick_base: TickBase::Second,
        queue: BinaryHeap::new(),
    };
    tq.push("world!", 10)
        .push("Hello, ", 300)
        .push("miao~", 20);

    tq.advance(500);
    assert_eq!(500, tq.now());
    assert_eq!(
        vec![("world!", 10), ("miao~", 20), ("Hello, ", 300)],
        tq.trigger().unwrap());
}