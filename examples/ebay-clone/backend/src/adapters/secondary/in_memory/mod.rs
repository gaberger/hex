//! In-memory marketplace adapter.
//!
//! A self-contained implementation of the write-side [`ReducerCallPort`] and the
//! read-side repository ports ([`ListingRepoPort`], [`BidRepoPort`],
//! [`AuctionRepoPort`]), all backed by a single shared `Arc<Mutex<Store>>`. It
//! stands in for the SpacetimeDB `marketplace` WASM module so the backend can
//! run and be acceptance-tested end-to-end without a live STDB, network, or
//! browser. The production profile swaps this for `stdb_client`; the port
//! contracts it satisfies are identical, which is the whole point of the
//! hexagonal boundary.
//!
//! It also ships trivial [`PasswordHasherPort`] / [`TokenIssuerPort`]
//! implementations so the auth use case can be wired without pulling argon2/JWT
//! into the in-memory profile. (Token == username; sufficient to prove the
//! verify-token → identity path.)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::Utc;

use crate::core::domain::{
    Bid, DomainError, Listing, ListingId, AuctionId, BidId, Money, Timestamp, User, UserId,
    WatchEntry,
};
use crate::core::ports::auction_repo::{Auction, AuctionRepoPort, Bid as AuctionBid};
use crate::core::ports::bid_repo::BidRepoPort;
use crate::core::ports::listing_repo::{ListingRepoError, ListingRepoPort, SearchListingsParams};
use crate::core::ports::password_hasher::PasswordHasherPort;
use crate::core::ports::reducer_call::{
    CreateListingInput, PlaceBidInput, ReducerCallPort, RegisterUserInput, WatchListingInput,
};
use crate::core::ports::token_issuer::TokenIssuerPort;

#[derive(Default)]
struct Store {
    users: HashMap<String, User>,
    next_user_id: u64,
    listings: Vec<Listing>,
    auctions: Vec<AuctionRec>,
    bids: Vec<BidRec>,
    next_listing_id: u64,
    next_auction_id: u64,
    next_bid_id: u64,
}

struct AuctionRec {
    id: AuctionId,
    listing_id: ListingId,
    seller: UserId,
    starting_price_cents: u64,
    end_time_unix: i64,
    /// (bidder, amount_cents, bid_id) of the current highest bid.
    highest: Option<(UserId, u64, u64)>,
}

struct BidRec {
    id: u64,
    listing_id: ListingId,
    bidder: UserId,
    amount_cents: u64,
}

/// Shared-state in-memory implementation of the marketplace ports.
///
/// Cheap to `clone()` — every clone points at the same backing `Store`, so the
/// same instance can be boxed behind several different port traits in the
/// composition root.
#[derive(Clone, Default)]
pub struct InMemoryMarketplace {
    store: Arc<Mutex<Store>>,
}

impl InMemoryMarketplace {
    pub fn new() -> Self {
        Self::default()
    }
}

fn rec_to_auction(a: &AuctionRec) -> Auction {
    let current_bid = a.highest.as_ref().map(|(bidder, amt, bid_id)| AuctionBid {
        id: BidId(*bid_id),
        bidder_id: bidder.clone(),
        amount: Money::try_from(*amt).unwrap_or(Money::try_from(0).unwrap()),
        timestamp: Timestamp::try_from(0).unwrap(),
    });
    Auction {
        id: a.id,
        listing_id: a.listing_id,
        current_bid,
        start_time: Timestamp::try_from(0).unwrap(),
        end_time: Timestamp::try_from(a.end_time_unix).unwrap_or(Timestamp::try_from(0).unwrap()),
    }
}

#[async_trait]
impl ReducerCallPort for InMemoryMarketplace {
    async fn register_user(&self, input: RegisterUserInput) -> Result<User, DomainError> {
        let mut s = self.store.lock().unwrap();
        if s.users.contains_key(&input.username) {
            return Err(DomainError::UsernameTaken(input.username));
        }
        s.next_user_id += 1;
        let user = User {
            id: s.next_user_id,
            canonical_username: input.username.clone(),
            created_at: Utc::now().naive_utc(),
        };
        s.users.insert(input.username, user.clone());
        Ok(user)
    }

    async fn create_listing(&self, input: CreateListingInput) -> Result<Listing, DomainError> {
        if input.starting_price == 0 {
            return Err(DomainError::InvalidStartingPrice);
        }
        let mut s = self.store.lock().unwrap();
        s.next_listing_id += 1;
        s.next_auction_id += 1;
        let listing_id = ListingId(s.next_listing_id);
        let auction_id = AuctionId(s.next_auction_id);
        let listing = Listing {
            id: listing_id.0.to_string(),
            title: input.title,
            description: input.description,
            starting_price_cents: input.starting_price,
            created_at: Utc::now().naive_utc(),
            auction_id: auction_id.0.to_string(),
        };
        s.auctions.push(AuctionRec {
            id: auction_id,
            listing_id,
            seller: input.user_id,
            starting_price_cents: input.starting_price,
            end_time_unix: input.end_time as i64,
            highest: None,
        });
        s.listings.push(listing.clone());
        Ok(listing)
    }

    async fn place_bid(&self, input: PlaceBidInput) -> Result<Bid, DomainError> {
        let mut s = self.store.lock().unwrap();
        let now = Utc::now().timestamp();
        let idx = s
            .auctions
            .iter()
            .position(|a| a.listing_id == input.listing_id)
            .ok_or_else(|| DomainError::Internal("listing not found".into()))?;
        {
            let a = &s.auctions[idx];
            if now >= a.end_time_unix {
                return Err(DomainError::AuctionEnded);
            }
            if a.seller == input.user_id {
                return Err(DomainError::SelfBidForbidden);
            }
            let min_required = match &a.highest {
                Some((_, amt, _)) => *amt + 1,
                None => a.starting_price_cents,
            };
            if input.amount < min_required {
                return Err(DomainError::BidTooLow);
            }
        }
        s.next_bid_id += 1;
        let bid_id = s.next_bid_id;
        s.auctions[idx].highest = Some((input.user_id.clone(), input.amount, bid_id));
        s.bids.push(BidRec {
            id: bid_id,
            listing_id: input.listing_id,
            bidder: input.user_id.clone(),
            amount_cents: input.amount,
        });
        Ok(Bid {
            bidder_id: input.user_id.0,
            amount_cents: input.amount,
        })
    }

    async fn watch_listing(&self, input: WatchListingInput) -> Result<WatchEntry, DomainError> {
        Ok(WatchEntry {
            auction_id: input.listing_id.0.to_string(),
            added_at: SystemTime::now(),
        })
    }
}

#[async_trait]
impl ListingRepoPort for InMemoryMarketplace {
    async fn get_listing_by_id(
        &self,
        listing_id: &ListingId,
    ) -> Result<Listing, ListingRepoError> {
        let s = self.store.lock().unwrap();
        s.listings
            .iter()
            .find(|l| l.id == listing_id.0.to_string())
            .cloned()
            .ok_or(ListingRepoError::NotFound)
    }

    async fn get_listings_by_criteria(
        &self,
        criteria: &SearchListingsParams,
    ) -> Result<Vec<Listing>, ListingRepoError> {
        let s = self.store.lock().unwrap();
        let mut out: Vec<Listing> = s
            .listings
            .iter()
            .filter(|l| match &criteria.query {
                Some(q) => l.title.to_lowercase().contains(q),
                None => true,
            })
            .cloned()
            .collect();
        if let Some(off) = criteria.offset {
            out = out.into_iter().skip(off as usize).collect();
        }
        if let Some(lim) = criteria.limit {
            out.truncate(lim as usize);
        }
        Ok(out)
    }
}

#[async_trait]
impl BidRepoPort for InMemoryMarketplace {
    async fn get_bid_by_id(&self, bid_id: BidId) -> Option<Bid> {
        let s = self.store.lock().unwrap();
        s.bids.iter().find(|b| b.id == bid_id.0).map(|b| Bid {
            bidder_id: b.bidder.0.clone(),
            amount_cents: b.amount_cents,
        })
    }

    async fn get_bids_by_listing_id(&self, listing_id: ListingId) -> Vec<Bid> {
        let s = self.store.lock().unwrap();
        s.bids
            .iter()
            .filter(|b| b.listing_id == listing_id)
            .map(|b| Bid {
                bidder_id: b.bidder.0.clone(),
                amount_cents: b.amount_cents,
            })
            .collect()
    }

    async fn get_bids_by_user_id(&self, user_id: UserId) -> Vec<Bid> {
        let s = self.store.lock().unwrap();
        s.bids
            .iter()
            .filter(|b| b.bidder == user_id)
            .map(|b| Bid {
                bidder_id: b.bidder.0.clone(),
                amount_cents: b.amount_cents,
            })
            .collect()
    }
}

#[async_trait]
impl AuctionRepoPort for InMemoryMarketplace {
    async fn get_auction(
        &self,
        auction_id: &AuctionId,
    ) -> Result<Option<Auction>, DomainError> {
        let s = self.store.lock().unwrap();
        Ok(s.auctions.iter().find(|a| a.id == *auction_id).map(rec_to_auction))
    }

    async fn list_active_auctions(&self) -> Result<Vec<Auction>, DomainError> {
        let now = Utc::now().timestamp();
        let s = self.store.lock().unwrap();
        Ok(s.auctions
            .iter()
            .filter(|a| a.end_time_unix > now)
            .map(rec_to_auction)
            .collect())
    }

    async fn list_recently_ended_auctions(
        &self,
        _start_time: Timestamp,
        _end_time: Timestamp,
    ) -> Result<Vec<Auction>, DomainError> {
        let now = Utc::now().timestamp();
        let s = self.store.lock().unwrap();
        Ok(s.auctions
            .iter()
            .filter(|a| a.end_time_unix <= now && a.highest.is_some())
            .map(rec_to_auction)
            .collect())
    }
}

/// Trivial password hasher for the in-memory profile (NOT for production).
pub struct PlainPasswordHasher;

#[async_trait]
impl PasswordHasherPort for PlainPasswordHasher {
    async fn hash_password(&self, password: String) -> Result<String, DomainError> {
        Ok(format!("plain:{password}"))
    }

    async fn verify_password(&self, password: String, hash: String) -> Result<bool, DomainError> {
        Ok(hash == format!("plain:{password}"))
    }
}

/// Token issuer that encodes the username directly (NOT for production).
pub struct UsernameTokenIssuer;

#[async_trait]
impl TokenIssuerPort for UsernameTokenIssuer {
    async fn issue(&self, user: &User) -> Result<String, DomainError> {
        Ok(user.canonical_username.clone())
    }

    async fn verify(&self, token: &str) -> Result<User, DomainError> {
        if token.is_empty() {
            return Err(DomainError::Internal("empty token".into()));
        }
        Ok(User {
            id: 0,
            canonical_username: token.to_string(),
            created_at: Utc::now().naive_utc(),
        })
    }
}
