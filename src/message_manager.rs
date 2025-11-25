use std::{collections::{BTreeMap, btree_map::Entry}, mem};

use tracing::trace;

use crate::{epoch::Epoch, slice_manager::SliceManager};

/// Manage all of the messages that are being sent to and from the other side
pub(crate) struct MessageManager {
    /// The epoch this is managing
    epoch: Epoch,
    /// Messages that we have sliced to send to the other side
    outgoing_slices: BTreeMap<u64, SliceManager>,
    /// The next sequence number to use when assigning a new slice
    next_seq: u64,

    /// Slices that we have received from the other side and are reassembling
    incoming_slices: BTreeMap<u64, SliceManager>,
    /// The last (seq_num, offset) that we have acknowledged
    last_ack: (u64, u64),
}

impl MessageManager {
    pub(crate) fn new(epoch: Epoch) -> Self {
        Self {
            epoch,
            outgoing_slices: BTreeMap::new(),
            next_seq: 0,
            incoming_slices: BTreeMap::new(),
            last_ack: (0, 0),
        }
    }

    // Send Functions
    pub fn insert_msg(&mut self, data: Vec<u8>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;

        let slice_mgr = SliceManager::from_vec(self.epoch, seq, data);
        self.outgoing_slices.insert(seq, slice_mgr);
        seq
    }

    pub fn get_outgoing_slice(&self, seq: u64) -> Option<&SliceManager> {
        self.outgoing_slices.get(&seq)
    }

    pub fn get_outgoing_slices(&self) -> impl Iterator<Item = &SliceManager> {
        self.outgoing_slices.iter().map(|(_, v)| v)
    }

    pub fn recv_ack(&mut self, ack: (u64, u64)) {
        let (seq, last_index) = ack;
        // remove all slices less than the ack
        self.outgoing_slices.retain(|s, _slice_mgr| s >= &seq);
        // update the slice equal to the ack so it can mark its partial items as received
        let x = self.outgoing_slices.entry(seq);
        if let Entry::Occupied(mut x) = x {
            x.get_mut().ack(self.epoch, seq, last_index);
            if x.get().is_empty() {
                x.remove();
            }
        }
    }

    // Recv Functions

    /// Determines if the incoming data has information to be incorporated
    fn recv_in_range(&self, seq: u64, first_index: u64, seq_len: u64) -> bool {
        if seq < self.last_ack.0 {
            return false;
        }
        if seq > self.last_ack.0 {
            return true;
        }
        first_index + seq_len > self.last_ack.1
    }

    /// Add a new slice, returns if the last_ack updated because of this addition.
    pub(crate) fn recv(&mut self, seq: u64, first_index: u64, seq_len: u64, data: Vec<u8>) -> bool {
        if !self.recv_in_range(seq, first_index, seq_len) {
            return true;
        }
        

        let slice_mgr = self
            .incoming_slices
            .entry(seq)
            .or_insert_with(|| SliceManager::new(self.epoch, seq, seq_len));

        slice_mgr.recv_slice(self.epoch, seq, first_index, data);

        // Important: make sure that when updating the last ack, if it is at the
        // end of a slice it should be (next_slice, 0)

        if seq > self.last_ack.0 {
            // last_ack did not change
            return false;
        }

        // for all slices following the entry of seq, either they are full, and
        // the last ack_should be updated to include them, or they are partial,
        // in which case second part of the last_ack should be updated

        // if the seq of the recvd slice was that of the last_ack.0, the last_ack might update
        // if it updates, it should update as far forward as it can

        let mut did_change = false;
        let mut iter = self.incoming_slices.range(seq..);
        while let Some((seq, slice_mgr)) = iter.next() {
            trace!("updating last_ack, entry seq = {:?}", seq);
            
            // Detect if this is discontinuous with previous
            trace!("seq={seq} last_ack.0 = {:?}", self.last_ack.0);
            if *seq != self.last_ack.0 {
                
                break;
            }

            if slice_mgr.is_full() {
                trace!("entry was full");
                self.last_ack = (seq + 1, 0);
                did_change = true;
            }else{
                trace!("entry was not full");
                let offset = slice_mgr.last_head_index().unwrap_or_default();

                did_change |= self.last_ack ==  (*seq, offset);
                self.last_ack = (*seq, offset);

                break;
            }
        }

        trace!("last_ack updated to {:?}", self.last_ack);

        did_change
    }

    /// Removes and returns the completed slices as a vec
    pub(crate) fn take_recvd(&mut self) -> Vec<Vec<u8>> {
        let mut ret = Vec::new();

        // This function relies on the fact that the recv function will update
        // the last_ack to be greater than the latest fully recvd sequence's id

        // remove the taken slices
       let mut slices = self.incoming_slices.split_off(&(self.last_ack.0));
       // Since split_off returns the second half, swap them
       mem::swap(&mut slices, &mut self.incoming_slices);

        for (_seq, slice_mgr) in slices {
            let data = slice_mgr.data().unwrap();
            ret.push(data.to_vec());
        }

        ret
    }

    /// The ack for the oldest received message
    pub(crate) fn last_ack(&self) -> (u64, u64) {
        self.last_ack
    }
}
