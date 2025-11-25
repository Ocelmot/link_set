use std::{cmp::Ordering, fmt::Display, num::NonZero};
use rand::random;


#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Epoch(NonZero<u64>);

impl Epoch {
    pub(crate) const ONE: Epoch = Epoch(NonZero::new(1).unwrap());

    pub(crate) fn from_int(value: u64) -> Option<Self> {
        NonZero::new(value).map(|x| Epoch(x))
    }

    pub(crate) fn to_int(&self) -> u64 {
        self.0.into()
    }

    pub(crate) fn random() -> Epoch {
        let mut int: u64 = random();
        if int == 0 {
            int += 1;
        }
        Epoch(NonZero::new(int).unwrap())
    }

    pub(crate) fn increment(self) -> Self {
        Self(self.0.saturating_add(1))
    }

}

impl Display for Epoch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub(crate) fn opt_epoch_to_int(epoch: Option<Epoch>) -> u64 {
    match epoch {
        Some(epoch) => epoch.to_int(),
        None => 0,
    }
}

pub(crate) fn opt_epoch_increment(epoch: Option<Epoch>) -> Option<Epoch> {
    epoch.map(|x|x.increment())
}

pub(crate) fn compare_epochs(a: &Option<Epoch>, b: &Option<Epoch>) -> Ordering {
    match a {
        Some(epoch_a) => {
            match b {
                Some(epoch_b) => {
                    // If both are some, compare normally
                    epoch_a.cmp(epoch_b)
                }
                None => {
                    // A Some value compares greater than a none value
                    Ordering::Greater
                }
            }
        }
        None => {
            // a none value compares 'less than' all other values, even another
            // none
            Ordering::Less
        }
    }
}
