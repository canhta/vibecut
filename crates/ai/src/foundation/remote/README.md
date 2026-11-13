# Remote provider admission

`remote` owns exact configuration/scope admission, disclosure construction and safe
provider-error classification. `defaults` is publicly exposed as `ai::defaults`:
the single shipped provider/feature model and known-rate table. `models` serves
`ai::list_remote_models`, the one bounded authenticated list request invoked only
by an explicit caller action. Limits, consent and pricing belong to the
[provider contract](../contracts/README.md#remote-model-settings).
