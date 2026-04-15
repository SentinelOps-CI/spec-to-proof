/-- Trivial examples (stdlib only; keeps CI fast). -/

theorem trivial_add_zero (n : Nat) : n + 0 = n :=
  Nat.add_zero n

theorem trivial_reflexivity (n : Nat) : n = n :=
  rfl

theorem trivial_implies (P : Prop) : P → P :=
  fun h => h
