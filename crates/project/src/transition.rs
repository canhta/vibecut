pub(crate) fn video_transition_defs() -> Vec<EffectDef> {
    use Badges::*;
    let m = A32;
    vec![
        tr("cross_dissolve", "Cross Dissolve", DISSOLVE, m, vec![]),
        tr("dip_to_black", "Dip to Black", DISSOLVE, m, vec![]),
        tr("dip_to_white", "Dip to White", DISSOLVE, m, vec![]),
    ]
}
