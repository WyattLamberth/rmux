#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."

# Pass decimal u64 seeds as arguments, or replay just RMUX_TEST_SEED.
valid_u64() {
  local digits=$1
  [[ $digits =~ ^[0-9]+$ ]] || return 1
  digits=${digits#"${digits%%[!0]*}"}
  digits=${digits:-0}
  [[ ${#digits} -lt 20 || ( ${#digits} -eq 20 && ! $digits > 18446744073709551615 ) ]]
}
steps=${RMUX_TEST_STEPS:-2000}
if ! valid_u64 "$steps" || [[ $steps =~ ^0+$ ]]; then
  echo 'RMUX_TEST_STEPS must be a positive decimal u64' >&2
  exit 2
fi
if (( $# > 0 )); then
  seeds=("$@")
elif [[ -n ${RMUX_TEST_SEED:-} ]]; then
  seeds=("$RMUX_TEST_SEED")
else
  seeds=(0 1 42 1337 18446744073709551615)
fi
for seed in "${seeds[@]}"; do
  if ! valid_u64 "$seed"; then
    echo "Seed must be a decimal u64: $seed" >&2
    exit 2
  fi
done
for seed in "${seeds[@]}"; do
  printf 'Universe replay: RMUX_TEST_SEED=%s RMUX_TEST_STEPS=%s\n' "$seed" "$steps"
  RMUX_TEST_SEED="$seed" RMUX_TEST_STEPS="$steps" \
    cargo test --workspace --locked --test universe -- --nocapture
done
