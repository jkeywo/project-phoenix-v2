//! Conservative integer stocks; claims retain physical quantity until consumed.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StockData")]
pub struct Stock {
    pub level: i64,
    pub ceiling: i64,
    pub reserved: BTreeMap<String, i64>,
    /// Promised receiving room, never held source quantity or spendable stock.
    pub incoming: BTreeMap<String, i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StockData {
    level: i64,
    ceiling: i64,
    #[serde(default)]
    reserved: BTreeMap<String, i64>,
    #[serde(default)]
    incoming: BTreeMap<String, i64>,
}
impl TryFrom<StockData> for Stock {
    type Error = String;
    fn try_from(data: StockData) -> Result<Self, String> {
        let stock = Self {
            level: data.level,
            ceiling: data.ceiling,
            reserved: data.reserved,
            incoming: data.incoming,
        };
        stock.validate()?;
        Ok(stock)
    }
}
impl Stock {
    pub fn new(level: i64, ceiling: i64) -> Result<Self, String> {
        let stock = Self {
            level,
            ceiling,
            reserved: BTreeMap::new(),
            incoming: BTreeMap::new(),
        };
        stock.validate()?;
        Ok(stock)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.level < 0 || self.ceiling < self.level {
            return Err("stock requires 0 <= level <= ceiling".into());
        }
        let mut total = 0i64;
        for (id, amount) in &self.reserved {
            if id.trim().is_empty() || *amount <= 0 {
                return Err("stock reservation requires a nonempty id and positive amount".into());
            }
            total = total
                .checked_add(*amount)
                .ok_or("stock reservations overflow")?;
        }
        if total > self.level {
            return Err("stock reservations exceed physical level".into());
        }
        let mut incoming = 0i64;
        for (id, amount) in &self.incoming {
            if id.trim().is_empty() || *amount <= 0 {
                return Err(
                    "incoming reservation requires a nonempty id and positive amount".into(),
                );
            }
            incoming = incoming
                .checked_add(*amount)
                .ok_or("incoming reservations overflow")?;
        }
        if self
            .level
            .checked_add(incoming)
            .is_none_or(|occupied| occupied > self.ceiling)
        {
            return Err("stock and incoming reservations exceed capacity".into());
        }
        Ok(())
    }
    pub fn reserved_total(&self) -> i64 {
        self.reserved
            .values()
            .copied()
            .fold(0i64, i64::saturating_add)
    }
    pub fn available(&self) -> i64 {
        self.level.saturating_sub(self.reserved_total()).max(0)
    }
    pub fn headroom(&self) -> i64 {
        self.ceiling
            .saturating_sub(self.level)
            .saturating_sub(self.incoming_total())
            .max(0)
    }
    pub fn incoming_total(&self) -> i64 {
        self.incoming
            .values()
            .copied()
            .fold(0i64, i64::saturating_add)
    }
    pub fn reserve_incoming(&mut self, id: &str, amount: i64) -> Result<(), String> {
        self.validate()?;
        if id.trim().is_empty() || amount <= 0 {
            return Err("incoming reservation needs a nonempty id and positive amount".into());
        }
        if let Some(existing) = self.incoming.get(id) {
            return if *existing == amount {
                Ok(())
            } else {
                Err("incoming claim already has a different quantity".into())
            };
        }
        if amount > self.headroom() {
            return Err("incoming reservation exceeds receiving room".into());
        }
        self.incoming.insert(id.into(), amount);
        Ok(())
    }
    pub fn release_incoming(&mut self, id: &str) -> Option<i64> {
        self.incoming.remove(id)
    }
    pub fn receive(&mut self, id: &str, amount: i64) -> Result<(), String> {
        self.validate()?;
        if amount <= 0 || self.incoming.get(id) != Some(&amount) {
            return Err("arrival does not match its incoming reservation".into());
        }
        let next = self
            .level
            .checked_add(amount)
            .ok_or("arrival quantity overflows")?;
        if next > self.ceiling {
            return Err("arrival exceeds stock capacity".into());
        }
        self.incoming.remove(id);
        self.level = next;
        Ok(())
    }
    pub fn reserve(&mut self, id: &str, amount: i64) -> Result<(), String> {
        self.validate()?;
        if id.trim().is_empty() || amount <= 0 {
            return Err("reservation needs a nonempty id and positive amount".into());
        }
        if let Some(existing) = self.reserved.get(id) {
            return if *existing == amount {
                Ok(())
            } else {
                Err(format!(
                    "reservation '{id}' already claims a different amount"
                ))
            };
        }
        if amount > self.available() {
            return Err(format!("stock cannot satisfy reservation '{id}'"));
        }
        self.reserved.insert(id.into(), amount);
        Ok(())
    }
    pub fn release(&mut self, id: &str) -> Option<i64> {
        self.reserved.remove(id)
    }
    pub fn consume(&mut self, id: &str) -> Option<i64> {
        let amount = self.reserved.remove(id)?;
        self.level -= amount;
        Some(amount)
    }
    /// Adjust unclaimed quantity only; existing claims cannot be stolen.
    /// Returns the effective change, which may be less than the requested delta.
    pub fn adjust(&mut self, delta: i64) -> i64 {
        let before = self.level;
        self.level = self.level.saturating_add(delta).clamp(
            self.reserved_total(),
            self.ceiling.saturating_sub(self.incoming_total()),
        );
        self.level - before
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRequirement {
    pub stock: usize,
    pub claim: String,
    pub amount: i64,
}

/// Claim every requirement, including repeated references to a stock, or none.
/// Only the stage being started should call this; queued future work is unclaimed.
pub fn reserve_requirements(
    stocks: &mut [Stock],
    requirements: &[ResourceRequirement],
) -> Result<(), String> {
    let mut staged = BTreeMap::<usize, Stock>::new();
    for requirement in requirements {
        let original = stocks
            .get(requirement.stock)
            .ok_or("requirement references an absent stock")?;
        let stock = staged
            .entry(requirement.stock)
            .or_insert_with(|| original.clone());
        stock.reserve(&requirement.claim, requirement.amount)?;
    }
    for (index, stock) in staged {
        stocks[index] = stock;
    }
    Ok(())
}

/// Units a transfer can move without stealing source claims or exceeding room.
/// The caller atomically commits both ends in its deterministic transaction.
pub fn plan_transfer(amount: i64, source_available: i64, destination_headroom: i64) -> i64 {
    amount
        .min(source_available)
        .min(destination_headroom)
        .max(0)
}

#[cfg(test)]
#[path = "stock_incoming_tests.rs"]
mod incoming_tests;

#[cfg(test)]
mod tests {
    use super::*;
    fn req(stock: usize, claim: &str, amount: i64) -> ResourceRequirement {
        ResourceRequirement {
            stock,
            claim: claim.into(),
            amount,
        }
    }
    #[test]
    fn transfer_is_conservative_for_empty_full_and_bounded_endpoints() {
        assert_eq!(plan_transfer(10, 7, 5), 5);
        assert_eq!(plan_transfer(10, 0, 5), 0);
        assert_eq!(plan_transfer(10, 7, 0), 0);
        assert_eq!(plan_transfer(-1, 7, 5), 0);
        assert_eq!(plan_transfer(i64::MAX, i64::MAX, 1), 1);
        let mut source = Stock::new(9, 9).unwrap();
        source.reserve("tube", 4).unwrap();
        let mut destination = Stock::new(1, 4).unwrap();
        let moved = plan_transfer(8, source.available(), destination.headroom());
        assert_eq!(source.adjust(-moved), -3);
        assert_eq!(destination.adjust(moved), 3);
        assert_eq!(source.level + destination.level, 10);
        assert_eq!(source.reserved.get("tube"), Some(&4));
    }
    #[test]
    fn claims_are_idempotent_but_conflicting_reuse_is_refused() {
        let mut stock = Stock::new(5, 8).unwrap();
        stock.reserve("job", 3).unwrap();
        stock.reserve("job", 3).unwrap();
        assert_eq!(stock.available(), 2);
        assert!(stock.reserve("job", 2).is_err());
        assert!(stock.reserve("other", 3).is_err());
        assert_eq!(stock.adjust(-8), -2);
        assert_eq!(stock.available(), 0);
        assert_eq!(stock.release("job"), Some(3));
        assert_eq!(stock.available(), 3);
        assert_eq!(stock.adjust(100), 5);
        assert_eq!(stock.headroom(), 0);
    }
    #[test]
    fn all_requirements_commit_or_none_even_on_one_shared_store() {
        let mut stocks = [Stock::new(3, 3).unwrap(), Stock::new(1, 1).unwrap()];
        let before = stocks.clone();
        assert!(reserve_requirements(&mut stocks, &[req(0, "a", 2), req(0, "b", 2)]).is_err());
        assert_eq!(stocks, before);
        assert!(reserve_requirements(&mut stocks, &[req(0, "a", 2), req(1, "b", 2)]).is_err());
        assert_eq!(stocks, before);
        reserve_requirements(&mut stocks, &[req(0, "a", 2), req(1, "b", 1)]).unwrap();
        assert_eq!(stocks[0].consume("a"), Some(2));
        assert_eq!(stocks[0].consume("a"), None);
        assert_eq!(stocks[0].level, 1);
    }
    #[test]
    fn malformed_stock_and_claims_are_rejected_when_decoded() {
        for text in [
            r#"{ "level":-1,"ceiling":5 }"#,
            r#"{ "level":2,"ceiling":1 }"#,
            r#"{ "level":2,"ceiling":5,"reserved":{"x":3} }"#,
            r#"{ "level":2,"ceiling":5,"reserved":{"":1} }"#,
            r#"{ "level":2,"ceiling":5,"reserved":{"x":0} }"#,
        ] {
            assert!(serde_json::from_str::<Stock>(text).is_err());
        }
        let mut stock = Stock::new(4, 6).unwrap();
        stock.reserve("x", 2).unwrap();
        let decoded: Stock = serde_json::from_str(&serde_json::to_string(&stock).unwrap()).unwrap();
        assert_eq!(decoded, stock);
    }
}
