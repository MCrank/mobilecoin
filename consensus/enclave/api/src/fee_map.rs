// Copyright (c) 2018-2022 The MobileCoin Foundation

//! A helper object for maintaining a map of token id -> minimum fee.

use alloc::{collections::BTreeMap, format, string::String};
use core::{convert::TryFrom, iter::FromIterator};
use displaydoc::Display;
use mc_common::ResponderId;
use mc_crypto_digestible::{DigestTranscript, Digestible, MerlinTranscript};
use mc_transaction_core::{tokens::Mob, Token, TokenId};
use serde::{Deserialize, Serialize};

/// A thread-safe object that contains a map of fee value by token id.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct FeeMap {
    /// The actual map of token_id to fee.
    /// Since we hash this map, it is important to use a BTreeMap as it
    /// guarantees iterating over the map is in sorted and predictable
    /// order.
    map: BTreeMap<TokenId, u64>,

    /// Cached digest value, formatted as a string.
    /// (Suitable for appending to responder id)
    cached_digest: String,
}

impl Default for FeeMap {
    fn default() -> Self {
        let map = Self::default_map();
        let cached_digest = calc_digest_for_map(&map);

        Self { map, cached_digest }
    }
}

impl TryFrom<BTreeMap<TokenId, u64>> for FeeMap {
    type Error = Error;

    fn try_from(map: BTreeMap<TokenId, u64>) -> Result<Self, Self::Error> {
        Self::is_valid_map(&map)?;

        let cached_digest = calc_digest_for_map(&map);

        Ok(Self { map, cached_digest })
    }
}

impl FeeMap {
    /// Create a fee map from an unsorted iterator.
    pub fn try_from_iter(iter: impl IntoIterator<Item = (TokenId, u64)>) -> Result<Self, Error> {
        let map = BTreeMap::from_iter(iter);
        Self::try_from(map)
    }

    /// Append the fee map digest to an existing responder id, producing a
    /// responder id that is unique to the current fee configuration.
    pub fn responder_id(&self, responder_id: &ResponderId) -> ResponderId {
        ResponderId(format!("{}-{}", responder_id.0, self.cached_digest))
    }

    /// Get the fee for a given token id, or None if no fee is set for that
    /// token.
    pub fn get_fee_for_token(&self, token_id: &TokenId) -> Option<u64> {
        self.map.get(token_id).cloned()
    }

    /// Update the fee map with a new one if provided, or reset it to the
    /// default.
    pub fn update_or_default(
        &mut self,
        minimum_fees: Option<BTreeMap<TokenId, u64>>,
    ) -> Result<(), Error> {
        if let Some(minimum_fees) = minimum_fees {
            Self::is_valid_map(&minimum_fees)?;

            self.map = minimum_fees;
        } else {
            self.map = Self::default_map();
        }

        // Digest must be updated when the map is updated.
        self.cached_digest = calc_digest_for_map(&self.map);

        Ok(())
    }

    /// Check if a given fee map is valid.
    pub fn is_valid_map(minimum_fees: &BTreeMap<TokenId, u64>) -> Result<(), Error> {
        // All fees must be greater than 0.
        if let Some((token_id, fee)) = minimum_fees.iter().find(|(_token_id, fee)| **fee == 0) {
            return Err(Error::InvalidFee(*token_id, *fee));
        }

        // Must have a minimum fee for MOB.
        if !minimum_fees.contains_key(&Mob::ID) {
            return Err(Error::MissingFee(Mob::ID));
        }

        // All good.
        Ok(())
    }

    /// Iterate over all entries in the fee map.
    pub fn iter(&self) -> impl Iterator<Item = (&TokenId, &u64)> {
        self.map.iter()
    }

    /// Helper method for constructing the default fee map.
    pub fn default_map() -> BTreeMap<TokenId, u64> {
        let mut map = BTreeMap::new();
        map.insert(Mob::ID, Mob::MINIMUM_FEE);
        map
    }
}

fn calc_digest_for_map(map: &BTreeMap<TokenId, u64>) -> String {
    let mut transcript = MerlinTranscript::new(b"fee_map");
    transcript.append_seq_header(b"fee_map", map.len() * 2);
    for (token_id, fee) in map {
        token_id.append_to_transcript(b"token_id", &mut transcript);
        fee.append_to_transcript(b"fee", &mut transcript);
    }

    let mut result = [0u8; 32];
    transcript.extract_digest(&mut result);
    hex::encode(result)
}

/// Fee Map error type.
#[derive(Clone, Debug, Deserialize, Display, PartialEq, PartialOrd, Serialize)]
pub enum Error {
    /// Token `{0}` has invalid fee `{1}`
    InvalidFee(TokenId, u64),

    /// Token `{0}` is missing from the fee map
    MissingFee(TokenId),
}

#[cfg(test)]
mod test {
    use super::*;
    use alloc::{string::ToString, vec};

    /// Different fee maps/responder ids should result in different responder
    /// ids.
    #[test]
    fn different_fee_maps_result_in_different_responder_ids() {
        let fee_map1 = FeeMap::try_from_iter([(Mob::ID, 100), (TokenId::from(2), 2000)]).unwrap();
        let fee_map2 = FeeMap::try_from_iter([(Mob::ID, 100), (TokenId::from(2), 300)]).unwrap();
        let fee_map3 = FeeMap::try_from_iter([(Mob::ID, 100), (TokenId::from(30), 300)]).unwrap();

        let responder_id1 = ResponderId("1.2.3.4:5".to_string());
        let responder_id2 = ResponderId("3.1.3.3:7".to_string());

        assert_ne!(
            fee_map1.responder_id(&responder_id1),
            fee_map2.responder_id(&responder_id1)
        );

        assert_ne!(
            fee_map1.responder_id(&responder_id1),
            fee_map3.responder_id(&responder_id1)
        );

        assert_ne!(
            fee_map2.responder_id(&responder_id1),
            fee_map3.responder_id(&responder_id1)
        );

        assert_ne!(
            fee_map1.responder_id(&responder_id1),
            fee_map1.responder_id(&responder_id2)
        );
    }

    /// Invalid fee maps are rejected.
    #[test]
    fn invalid_fee_maps_are_rejected() {
        let test_token_id = TokenId::from(2);

        // Missing MOB is not allowed
        assert_eq!(
            FeeMap::is_valid_map(&BTreeMap::default()),
            Err(Error::MissingFee(Mob::ID)),
        );

        assert_eq!(
            FeeMap::is_valid_map(&BTreeMap::from_iter(vec![(test_token_id, 100)])),
            Err(Error::MissingFee(Mob::ID)),
        );

        // All fees must be >0
        assert_eq!(
            FeeMap::is_valid_map(&BTreeMap::from_iter(vec![(Mob::ID, 0)])),
            Err(Error::InvalidFee(Mob::ID, 0)),
        );

        assert_eq!(
            FeeMap::is_valid_map(&BTreeMap::from_iter(vec![
                (Mob::ID, 10),
                (test_token_id, 0)
            ])),
            Err(Error::InvalidFee(test_token_id, 0)),
        );
    }
}
