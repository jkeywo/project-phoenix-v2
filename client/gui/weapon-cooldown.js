/** Join static, authoritative bank durations onto live states by bank id.
 * Never mutate the cached blackboard or infer a duration from observed samples.
 */
export function withWeaponCooldowns(banks, configs = []) {
  const durations = new Map(configs
    .filter(config => Number.isFinite(config.cooldown_secs) && config.cooldown_secs > 0)
    .map(config => [config.id, config.cooldown_secs]));
  return banks.map(bank => durations.has(bank.id)
    ? { ...bank, cooldown_secs: durations.get(bank.id) }
    : bank);
}

/** Remaining cooldown as a bounded percentage, or unknown without a duration.
 * Presentation follows authoritative snapshots; elapsed wall time never fires
 * a weapon or advances its cooldown on the client.
 */
export function cooldownRemainingPercent(bank) {
  if (!Number.isFinite(bank.cooldown_remaining)
      || !Number.isFinite(bank.cooldown_secs) || bank.cooldown_secs <= 0) return null;
  return Math.max(0, Math.min(100, bank.cooldown_remaining / bank.cooldown_secs * 100));
}
