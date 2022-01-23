// Types and helpers that support running differential dataflow computations.
// Dataflow types are very generic to allow for all sorts of usage scenarios.
// These aliases for the Elm-pair use case should be a bit easier to work with.

use differential_dataflow::trace::TraceReader;
use timely::progress::frontier::AntichainRef;

pub type Timestamp = u32;

pub type Diff = isize;

pub type Allocator = timely::communication::allocator::Thread;

pub type Worker =
    timely::worker::Worker<timely::communication::allocator::thread::Thread>;

pub type Input<A> =
    differential_dataflow::input::InputSession<Timestamp, A, Diff>;

pub type Collection<'a, A> =
    differential_dataflow::collection::Collection<Scope<'a>, A, Diff>;

pub type Scope<'a> = timely::dataflow::scopes::child::Child<
    'a,
    timely::worker::Worker<Allocator>,
    Timestamp,
>;

pub type Probe = timely::dataflow::operators::probe::Handle<Timestamp>;

pub type KeyTrace<K, V> = differential_dataflow::operators::arrange::TraceAgent<
    differential_dataflow::trace::implementations::spine_fueled::Spine<
        K,
        V,
        Timestamp,
        Diff,
        std::rc::Rc<
            differential_dataflow::trace::implementations::ord::OrdValBatch<
                K,
                V,
                Timestamp,
                Diff,
            >,
        >,
    >,
>;

pub type SelfTrace<K> = differential_dataflow::operators::arrange::TraceAgent<
    differential_dataflow::trace::implementations::spine_fueled::Spine<
        K,
        (),
        Timestamp,
        Diff,
        std::rc::Rc<
            differential_dataflow::trace::implementations::ord::OrdKeyBatch<
                K,
                Timestamp,
                Diff,
            >,
        >,
    >,
>;

#[allow(clippy::type_complexity)]
pub struct Cursor<T: TraceReader> {
    pub cursor: T::Cursor,
    pub storage: <T::Cursor as differential_dataflow::trace::Cursor<
        T::Key,
        T::Val,
        T::Time,
        T::R,
    >>::Storage,
}

impl<T: TraceReader> Cursor<T> {
    pub fn iter(&mut self) -> CursorKeys<T> {
        differential_dataflow::trace::cursor::Cursor::rewind_keys(
            &mut self.cursor,
            &self.storage,
        );
        differential_dataflow::trace::cursor::Cursor::rewind_vals(
            &mut self.cursor,
            &self.storage,
        );
        CursorKeys {
            cursor: &mut self.cursor,
            storage: &self.storage,
        }
    }
}

#[allow(clippy::type_complexity)]
pub struct CursorKeys<'a, T: TraceReader> {
    cursor: &'a mut T::Cursor,
    storage: &'a <T::Cursor as differential_dataflow::trace::Cursor<
        T::Key,
        T::Val,
        T::Time,
        T::R,
    >>::Storage,
}

impl<'a, T> Iterator for CursorKeys<'a, T>
where
    T: TraceReader<R = Diff, Time = Timestamp>,
    T::Key: 'a,
{
    type Item = &'a T::Key;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(module) =
            differential_dataflow::trace::cursor::Cursor::get_key(
                self.cursor,
                self.storage,
            )
        {
            let mut times = 0;
            differential_dataflow::trace::cursor::Cursor::map_times(
                self.cursor,
                self.storage,
                |_, r| times += r,
            );
            differential_dataflow::trace::cursor::Cursor::step_key(
                self.cursor,
                self.storage,
            );
            if times > 0 {
                return Some(module);
            }
        }
        None
    }
}

// Advancing a dataflow calculation until there is no work left to be done
// requires handling in a particular way all the calculation's inputs,
// aggregates, and probes. This trait allows you to pass inputs, aggregates, and
// probes in one giant tuple, and then figures out the rest.
//
//     Advancable::advance(
//         &mut (
//             some_input,
//             other_input,
//             aggregate,
//             probes,
//         ),
//         worker,
//     );
pub trait Advancable {
    fn advance_self(&mut self, time: Timestamp);

    fn get_time(&self) -> Option<Timestamp>;

    fn not_caught_up(&self, time: Timestamp) -> bool;

    fn advance(&mut self, worker: &mut Worker) {
        // This `.unwrap()` will trigger if there's no probes in the mix. That
        // will only happen at development time and be very visible.
        let next_time = 1 + self.get_time().unwrap();
        self.advance_self(next_time);
        worker.step_while(|| self.not_caught_up(next_time));
    }
}

impl<A: differential_dataflow::Data> Advancable for Input<A> {
    fn advance_self(&mut self, time: Timestamp) {
        self.advance_to(time);
        self.flush();
    }

    fn get_time(&self) -> Option<Timestamp> {
        Some(*self.time())
    }

    fn not_caught_up(&self, _: Timestamp) -> bool {
        false
    }
}

impl<Tr: TraceReader<Time = Timestamp, R = Diff>> Advancable
    for differential_dataflow::operators::arrange::agent::TraceAgent<Tr>
{
    fn advance_self(&mut self, time: Timestamp) {
        self.set_logical_compaction(AntichainRef::new(&[time]));
        self.set_physical_compaction(AntichainRef::new(&[time]));
    }

    fn get_time(&self) -> Option<Timestamp> {
        None
    }

    fn not_caught_up(&self, _: Timestamp) -> bool {
        false
    }
}

impl Advancable for Probe {
    fn advance_self(&mut self, _: Timestamp) {}

    fn get_time(&self) -> Option<Timestamp> {
        None
    }

    fn not_caught_up(&self, time: Timestamp) -> bool {
        self.less_than(&time)
    }
}

impl<A: Advancable> Advancable for Vec<A> {
    fn advance_self(&mut self, time: Timestamp) {
        self.iter_mut().for_each(|a| a.advance_self(time))
    }

    fn get_time(&self) -> Option<Timestamp> {
        self.iter().filter_map(|a| a.get_time()).max()
    }

    fn not_caught_up(&self, time: Timestamp) -> bool {
        self.iter().map(|a| a.not_caught_up(time)).any(|b| b)
    }
}

macro_rules! advancable_tuple {
    (@not_caught_up $time:expr, $head:ident, $( $tail:ident,)* ) => {
        $head.not_caught_up($time)
            $(|| $tail.not_caught_up($time))*
    };

    (( $( $name:ident),+ )) => {
        #[allow(non_snake_case)]
        impl<$($name: Advancable,)+> Advancable for ($(&mut $name,)+) {
            fn advance_self(&mut self, time: Timestamp) {
                let ($($name,)+) = self;
                $($name.advance_self(time);)+
            }

            fn get_time(&self) -> Option<Timestamp> {
                let ($($name,)+) = self;
                [$($name.get_time(),)+].iter().flatten().max().copied()
            }

            fn not_caught_up(&self, time: Timestamp) -> bool {
                let ($($name,)+) = self;
                advancable_tuple!(@not_caught_up time, $($name,)+)
            }
        }
    };
}

advancable_tuple!((A, B));
advancable_tuple!((A, B, C));
advancable_tuple!((A, B, C, D));
advancable_tuple!((A, B, C, D, E));
advancable_tuple!((A, B, C, D, E, F));
advancable_tuple!((A, B, C, D, E, F, G));
advancable_tuple!((A, B, C, D, E, F, G, H));
advancable_tuple!((A, B, C, D, E, F, G, H, I));
advancable_tuple!((A, B, C, D, E, F, G, H, I, J));
