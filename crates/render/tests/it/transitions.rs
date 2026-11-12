#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::support::{Empty, Hooks};
use project::*;
use render::*;
use render::{Error, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use time::{SourceRange, Tick};

/// One row per catalogue kind: a 4x4 grid of mean 8-bit RGB at progress 0.25, 0.5 and 0.75.
const ORACLE: &[(&str, [&str; 3])] = &[
    (
        "cross_dissolve",
        [
            "906e7cac6e7cc6747cd66e7c8f8385ab8385d7a285d5838590948dac948dc99d8dd6948d90a294aba294c2a294d6a294",
            "7d7693937693ac7e93b676937c87a09287a0d3bda0b587a07c96ac9396acb4a6acb596ac7ca3b892a3b8a5a3b8b5a3b8",
            "647ca5737ca58b88a58b7ca5638bb6728bb6ced4b68a8bb66398c67298c699afc68b98c663a4d472a4d47fa4d48aa4d4",
        ],
    ),
    (
        "dip_to_black",
        [
            "7549428d4942a04942b14942745b428c5b42a05b42b15b427569428d6942a06942b169427475428c7542a07542b17542",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "2c5e842c5e843b68842c5e842c67932c679391aa932c67932c70a02c70a04d85a02c70a02c77ad2c77ad2c77ad2c77ad",
        ],
    ),
    (
        "dip_to_white",
        [
            "d7c6c5e3c6c5eec6c5f8c6c5d7ccc5e2ccc5eeccc5f8ccc5d7d1c5e3d1c5eed1c5f8d1c5d7d7c5e2d7c5eed7c5f8d7c5",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "c0cddec0cddec5d2dec0cddec0d0e6c0d0e6e6f4e6c0d0e6c0d4eec0d4eecce0eec0d4eec0d8f5c0d8f5c0d8f5c0d8f5",
        ],
    ),
    (
        "dip_to_color",
        [
            "c0524ccd524cda524ce5524cbf624ccd624cda624ce5624cc06f4ccd6f4cda6f4ce56f4cbf7a4ccd7a4cda7a4ce57a4c",
            "d93333d93333d93333d93333d93333d93333d93333d93333d93333d93333d93333d93333d93333d93333d93333d93333",
            "a46488a46488aa6e88a46488a46d97a46d97d2ad97a46d97a475a4a475a4b389a4a475a4a47cb0a47cb0a47cb0a47cb0",
        ],
    ),
    (
        "light_fade",
        [
            "a6889fc5889fe3909ff4889fa59facc49facf9ceacf39faca6b2b7c5b2b7e8c0b7f4b2b7a5c3c2c4c3c2dec3c2f3c3c2",
            "aba2c9c9a2c9e6adc9f7a2c9aab9dac8b9dafdf9daf5b9daabcdebc9cdebeadeebf6cdebaadef8c8def8e1def8f5def8",
            "8393bf9893bfb6a0bfba93bf83a6d298a6d2f6f5d2b9a6d283b6e398b6e3c3cde3bab6e383c4f398c4f3aac4f3b9c4f3",
        ],
    ),
    (
        "film_fade",
        [
            "896e73a16e73ba7173c66e73888379a18379d69979c6837989937da1937dc09b7dc6937d88a282a1a282b5a282c6a282",
            "717489817489997c899a7489708794818794d1b4949a879471969d81969da4a49d9a969d70a3a681a3a68ea3a69aa3a6",
            "587b9f607b9f76869f6d7b9f588baf608bafccceaf6d8baf5898bc6098bc88adbc6d98bc58a4c960a4c967a4c96da4c9",
        ],
    ),
    (
        "soft_focus_fade",
        [
            "a46965c16962db6962f0695fa17e61c17e63db7e63f17e63a29163c19163db9163f09162a2a162c1a163dba163f0a162",
            "817b98988197b08a97b781968088a1998fa1b499a1b88ea18095ac989aacb0a2acb79aac809fb595a1b4aaa4b4b5a1b4",
            "3f83b64485b66d9db64385b63f8fc95097c9afd4c94d95c93f9adb469ddb7fbbdb459cdb3fa4ea3fa4ea42a5ea3fa4ea",
        ],
    ),
    (
        "burn_through",
        [
            "a1675dc1675ddb675df1675da07e5dc07e5dda7e5cf07e5da0915dc1915ddb915df1915ca0a15dbfa15cdaa25df0a15c",
            "c3836ebe8b87d58f809d96b4dd9a79d8a4a3dabe8ca79cad95a1baa0abdae6aa8d78a3d77fabd575ace2a5b5de8aadcc",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "brightness_fade",
        [
            "a1675dc1675ddb675df1675da07e5dc07e5dda7e5df07e5da1915dc1915ddb915df1915da0a25dc0a25ddba25df0a25f",
            "8f6e788e75928789a55780b06889a75b8cbbc7e4c5418ec84b99d3419ada6bb6db3f9adb40a4ea3fa4eb3fa4eb3fa4eb",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "pixel_blocks",
        [
            "bb7f7cc87f7ce67f7cf57f7cbb7f7cc87f7ce67f7cf57f7cbb7f7cc87f7ce67f7cf57f7c9fa93faea93fd2a93fe6a93f",
            "3f92cf3f92cfe78563e785633f92cf3f92cfe78563e785633f92cf3f92cfe78563e785636f9ca96f9ca99c95a39c95a3",
            "3f8fca6dadcad3f0ca76b3ca3f8fca6dadcad3f0ca76b3ca3f8fca6dadcad3f0ca76b3ca3fa9f33fa9f33fa9f33fa9f3",
        ],
    ),
    (
        "straight_wipe",
        [
            "4580adc1675ddb675df1675d478dc3c07e5dda7e5df07e5d4699d2c1915ddb915df1915d46a4e2c0a25ddba25df1a25d",
            "3f82b43f82b4db675df1675d3f8fc93f8fc9da7e5df07e5d3f9adb3f9adbdb915df1915d3fa4eb3fa4ebdba25df1a25d",
            "3f82b43f82b45390b4e668613f8fc93f8fc9c7e8c9e67f653f9adb3f9adb6bb6dbe692653fa4eb3fa4eb3fa4ebe6a267",
        ],
    ),
    (
        "soft_edge_wipe",
        [
            "a1675ec1675ddb675df1675d9f7f5fc07e5dda7e5df07e5da0925fc1915ddb915df1915da0a260c0a25ddba25df1a25d",
            "667ba3897798b57b8bd46f7d668ab58a88a8d5b599d383886698c38996b5bba4a3d4948f66a4d28aa3c1afa3add3a298",
            "3f82b43f82b45390b44382b43f8fc93f8fc9c7e8c9448ec83f9adb3f9adb6bb6db439ada3fa4eb3fa4eb3fa4eb43a4ea",
        ],
    ),
    (
        "clock_wipe",
        [
            "a1675dc1675d5390b43f82b4a07e5dc07e5dcadcb98789a0a1915dc1915ddb915df1915da0a25dc0a25ddba25df1a25d",
            "a1675dc1675d5390b43f82b4a07e5dc07e5dc7e8c93f8fc9a1915dc1915d6db6da3f9adba0a25dc0a25d44a4e73fa4eb",
            "a1675dc1675d5390b43f82b48e8174b97f63c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "corner_sweep",
        [
            "81717e4880ac5390b43f82b4a07e5db38063c8c6a33f8fc9a1915dc1915ddb915db4958ea0a25dc0a25ddba25df1a25d",
            "5a7ba03f82b45390b43f82b48b816a408ec8c7e8c93f8fc9a1915d7297a96bb6db3f9adba0a25db4a26648a4e53fa4eb",
            "487fad3f82b45390b43f82b45789a23f8fc9c7e8c93f8fc971959d3f9adb6bb6db3f9adb83a27f3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "star_wipe",
        [
            "a1675db36969cc6a65f1675d9f7e5d6988a2d4c6a7f07e5da1915d7b96a4a4a69af1915da0a25dafa26ec7a270f1a25d",
            "7b70825d7ba976899bad7080578aa63f8fc9c7e8c9758aaa95936f3f9adb6bb6dbdc936993a26f40a4ea40a4eadca26e",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc95198bc3f9adb6bb6db6998c05fa3b23fa4eb3fa4eb87a3b7",
        ],
    ),
    (
        "panel_wipe",
        [
            "3f82b49a6f6ddb675df1675d5b8bb0bf7f5eda7e5df07e5d92936bc1915ddb915df1915da0a25dc0a25ddba25df1a25d",
            "3f82b43f82b46389a6f1675d3f8fc93f8fc9d9a386f07e5d3f9adb6d97b1db915df1915d3fa4ebb2a274dba25df1a25d",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c95a8cba3f9adb3f9adb6db6dac494743fa4eb3fa4eb6ea4bff1a25d",
        ],
    ),
    (
        "stair_wipe",
        [
            "6f7480c1675ddb675df1675d558cbcc07e5eda7e5df07e5d5399c6c0925edb915df1915d74a3a7c0a25ddba25df1a25d",
            "3f82b45c7c9cdb675df1675d3f8fc9418ec8da8b6bf07e5d3f9adb419adad19869f1915d3fa4eb5fa4ccdaa25ef1a25d",
            "3f82b43f82b45790b3e768643f8fc93f8fc9c7e8c9b2848d3f9adb3f9adb6bb6dbb095893fa4eb3fa4eb42a4eae7a269",
        ],
    ),
    (
        "stretch_wipe",
        [
            "71789a9f7287cb6c72ee68617289ab9f8595ca817bed7f657197b89f959ecb937fee926572a3c59fa3a8caa286eda267",
            "3f82b43f82b44881b3727eab3f8fc93f8fc97fb0c798abbe3f9adb3f9adb489ad97399ce3fa4eb3fa4eb48a4e973a4de",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "neon_edge_wipe",
        [
            "6bd6eec2979cdb736ef1696171ddf6c1a79fdb8870f080616de3f6c2b59ddb9a6ef193616fe8f7c2c19edba96ff1a361",
            "4091c249c4eaddb6bcf27b78409dd549cbf4ddc2c1f18e7b40a7e649d2fbddcdbef19f7940b0f549d8feddd7bff1ae7a",
            "3f84b6408bbc57b5d8bbd1dc3f90ca4097d0c8f8e8bfd9e43f9bdc40a1e16fd1f5bde1df3fa6ec40abf143c3fdbee8e1",
        ],
    ),
    (
        "circle_reveal",
        [
            "a1675dc1675ddb675df1675da07e5d8c8481daa78bf07e5da1915d919593b8a384f1915da0a25dc0a25ddba25df1a25d",
            "a1675d637aa57d8898f1675d9480673f8fc9c7e8c9e0806997926c3f9adb6bb6dbe09268a0a25d61a4c86aa4c2f1a25d",
            "6c76993f82b45390b48e7684518aaa3f8fc9c7e8c96e8ab35598bd3f9adb6bb6db6d98ba66a3ac3fa4eb3fa4eb91a3ac",
        ],
    ),
    (
        "box_reveal",
        [
            "a1675dc1675ddb675df1675da07e5da58173da9473f07e5da1915da8937bcfa06ff1915da0a25dc0a25ddba25df1a25d",
            "a1675dc1675ddb675df1675da07e5d3f8fc9c7e8c9f07e5da1915d3f9adb6bb6dbf1915da0a25dc0a25ddba25df1a25d",
            "826e7e797490998290ba6e756187923f8fc9c7e8c99287a16896a83f9adb6bb6db8f969f7ca38179a4aa84a4aabda390",
        ],
    ),
    (
        "diamond_reveal",
        [
            "a1675dc1675ddb675df1675da07e5d848588d9ae93f07e5da1915d889698bcad8df1915da0a25dc0a25ddba25df1a25d",
            "a1675d817392a18183f1675d69868f3f8fc9c7e8c99b869b6f969f3f9adb6bb6db999697a0a25d7da3a290a3a5f1a25d",
            "6b75983f82b45390b48c75893f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb66a4ad3fa4eb3fa4eb8ea4ac",
        ],
    ),
    (
        "cross_reveal",
        [
            "a1675d9d6e7cba7471f1675d81827d5d8aa6d2cdb5be827f8594875f98b697b4bbbc947ba0a25d98a37db4a38bf1a25d",
            "a1675d3f82b45390b4f1675d3f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adba0a25d3fa4eb3fa4ebf1a25d",
            "577ba83f82b45390b4687b9a3f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb56a4cf3fa4eb3fa4eb68a4c4",
        ],
    ),
    (
        "double_doors",
        [
            "a1675d88738e9e7979f1675da07e5d818585d9ae94f07e5da1915d869599aba290f1915da0a25d83a39a99a39df1a25d",
            "a1675d3f82b45390b4f1675da07e5d3f8fc9c7e8c9f07e5da1915d3f9adb6bb6dbf1915da0a25d3fa4eb3fa4ebf1a25d",
            "6b76983f82b45390b48d76846187923f8fc9c7e8c99287a16896a83f9adb6bb6db8f969f64a3ab3fa4eb3fa4eb90a3ae",
        ],
    ),
    (
        "corner_inset",
        [
            "4a7fa8c1675ddb675df1675da07e5dc07e5dda7e5df07e5da1915dc1915ddb915df1915da0a25dc0a25ddba25df1a25d",
            "3f82b43f82b4db675df1675d3f8fc93f8fc9da7e5df07e5da1915dc1915ddb915df1915da0a25dc0a25ddba25df1a25d",
            "3f82b43f82b45390b4e668613f8fc93f8fc9c7e8c9e67f653f9adb3f9adb6bb6dbe692659ba264b9a264d2a264f0a25e",
        ],
    ),
    (
        "zoom_cross",
        [
            "b0735ec77365d5735be87363b18261c78263d5825ae7825fb18f61c08f52dc8f6be78f5fb29b61c99b6bd39b53e79b5f",
            "9888a4a289a8ccc29ad4c39e918ca2968da1d3c3a9d8c6a79091a59291a1c6b5afd8c9ac9095a99295a5ab9bb2c6b7af",
            "3f88be3f88bea9cebe69a1be3f90cc4091ccd1f0cc84bacc3f98d83f98d88fcad85aa7d83fa0e43fa0e43fa0e43fa0e4",
        ],
    ),
    (
        "zoom_smear",
        [
            "a76c5fc26c5eda6c60ee6c62a98063c38060d8805cec805fa6905ec2905cda9062ee9064aa9f64c49f63d89f5cec9f5d",
            "827a96947a96b89396b47c968388a29588a2d4c2a1b992a18195ab9395abbaacadb497ac83a0b695a0b6a3a0b4b1a0b4",
            "3f84b93f84b977a8b94387b93f8fca3f8fcad0eeca559cca3f99da3f99da7cbfda429bda3fa2e83fa2e83fa2e83fa2e8",
        ],
    ),
    (
        "spin_smear",
        [
            "a1695fc16860db6862f16964a37f64c17f62da7e5ef07e60a29161c1915fdb9161f19163a2a161c1a161dba161f0a062",
            "7c7893937693ae8294b577967e87a09589a1d0bba0b487a17d96ab9396acb5a7adb595ae7ea2b693a2b7a6a2b8b5a2b7",
            "3f83b63f82b55b94b53f83b63f8ec94290c9c5e6c93f8fc93f9adb3f9adb75b9db3f9adb3fa3ea3fa4eb3fa4eb3fa4ea",
        ],
    ),
    (
        "streak_smear",
        [
            "b9676cc46768ce6763d8675db17e5abf7e5fcd7e62d97e66b69166c29165ce9163d89160b3a260c1a262cda262d9a263",
            "8c77969377959977949f77938b8d9e938d9f9b8da0a38da18b98ae9298ad9998ad9f98ac86a3b78fa3b796a3b89da3b8",
            "4484b44484b44484b44484b45e9dc95e9dc95e9dc95e9dc94a9edb4a9edb4a9edb4a9edb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "lens_defocus",
        [
            "a36865c16862db6862f0685ea17e61c17e63db7e63f17e64a29163c19163db9163f09161a2a162c1a163dba163f1a163",
            "7f7896967d95b18995b77e947f86a19a8fa0b59ba0b98da07e95ac979aacb3a5acb899ac7fa1b794a2b6aaa6b6b4a1b5",
            "3f82b54184b56a9bb54183b53f8ec94e96c9b2d6c94b95c93f9adb449cdb81bcdb439bdb3fa4eb3fa4eb40a4eb3fa4eb",
        ],
    ),
    (
        "bloom",
        [
            "f2a194fea192ffa191ffa18feebc93febc92ffbc92ffbc90f0d592fed592ffd592ffd591efe993fee992ffe991ffe990",
            "c5bce7e7c2e6fecfe6ffc2e5c4d1f7e8d7f6fefbf6ffd7f6c4e6fee7eafefef5feffeafec4f6ffe4f8fffcfafffff8ff",
            "61c4ff75cdff9addff74cdff61d3ff7addffecfdff79ddff61e2ff75eaffa8f7ff74e9ff60eeff6af1ff75f6ff69f1ff",
        ],
    ),
    (
        "flash",
        [
            "a36b62c26b62db6b62f16b62a18162c18162db8162f18162a29462c29462db9462f19462a2a362c1a362dba362f1a362",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "4785b64785b65a92b64785b64791ca4791cac7e8ca4791ca479cdc479cdc71b7dc479cdc47a6ec47a6ec47a6ec47a6ec",
        ],
    ),
    (
        "lens_flare",
        [
            "c38871dc876edd6a5ef1675dcca979efb184de8360f07e5db9a36cd7a46ce39b64f49560a6a560c6a561dba25df1a35e",
            "827994a88699be8e98b77793858da2ccb3b3f3d7b0b98aa18298adc6b9bcdfc9bebd9baf7ea4b8a7afbcb8afbcb9a6b9",
            "4283b54183b56194b64e86b64b92ca5c98ccddf2cf74a0ce3f9adb4c9ddcc1d7e89bbde63fa4eb47a6ec82b8f183b9f2",
        ],
    ),
    (
        "light_leak",
        [
            "eb9c7ddb7a65e16b5ef2685df6b081e09067e2835ef17f5df1ba7be0a065e3955ef2925de6be73ddad63e2a55ef2a25d",
            "ffe5d0f6bbb4d0949bbd7a94fff5dcfdccc1f4cea9bf8ba1fff2dcfbd0c6deb9b3c09aadfee3d7f3ccc8cdb2bcbfa6b8",
            "d0c7d8f9dde3e7ced4959dbfe1d5ebffecf5fcfae7a3abd3e0d7f5feebfdf4e3f2a2b2e2ced0fbf2dffedbd0f993b5f0",
        ],
    ),
    (
        "colour_leak",
        [
            "fbbfa7fcd3bff6c0bbf78a84fdcfaffee1c3f9cebef69c89fcd0acfcdab4f5c8a9f6a47bf0c9a0f4cc9beebe8bf4aa6e",
            "d29eb9f6c1bcf9dbc5e6d9dce1afcdfccfd0fefdd6eae9ece5b7dafdd1dbfae8dae4e1e7dcbbe0f1cbdfebd2d8d7ced8",
            "808ac49e98ccbfbccbbec7c89198ddb2a6e5fafae4c8d3df9ba4f0bbb0f9d8d5f6c1d2f09aadfdb8b5ffbdc1feadc9fb",
        ],
    ),
    (
        "colour_burn",
        [
            "ee8f9afe919cff939dff959ee8ac9cfeae9dffaf9effb19fefc49efec69fffc7a0ffc8a1eed99ffedaa0ffdba1ffdca2",
            "b0c5eccac6ece5cdedf4c8edb3e0facde1fafdfcfcf5e3fab7f7ffd0f8ffeefcfff8f9ffbbffffd3ffffe9fffff9ffff",
            "77d5e777d5e789d9e877d5e77ce8fd7ce8fdf4fdfe7ce8fd80f9ff80f9ffa7fcff80f9ff84ffff84ffff84ffff84ffff",
        ],
    ),
    (
        "light_sweep",
        [
            "c79c90c77268db685ef1675dd3b79fca8c6fdb805ff07e5ddfd0abcea274dc9460f1925debe6bbd3b67edda563f1a25d",
            "f9fdfffffffffffefcffdbccf1fbffffffffffffffffefdde5f7fffffffffffffffffbe9d5f3fffefffffffffffffef4",
            "4082b54886b781a9c4bfd5eb3f8fc94591cad5f2d3adcef03f9adb439bdc81bfe19bcaf63fa4eb41a5ec53abef8bc7fb",
        ],
    ),
    (
        "light_rays",
        [
            "a1675dc3685fdf6960fc6f67a2805fc98463e98866fd8f6dac9966d29e66f7a671feab73b7b36ce2ba73f8be70ffc477",
            "7d7693937693dab3b3c98b9f7d87a1968aa3fefce2dfb1be859eb69ea1b9ded4dcd6b4c98eb6ceabbad3c0bdd4d6c1d9",
            "3f82b43f82b4b8dbe46fa7c73f8fc9408fc9fdfefe8dc7e541a0e341a0e3a5d8f761b5ec45b3fc45b3fc45b3fc45b3fc",
        ],
    ),
    (
        "phosphor_trail",
        [
            "a79979c6a07cdfa87ff4ad82a8b17fc7b782e4c185f4c188acc585c9ca88e3d18bf6d38dadd78bcbdb8de3e090f6e392",
            "828b98988f99b29a9abb959b839ea799a1a8d8d2a8bba7a985aeb49bb1b4bbc2b5bdb6b686bdbf9cbfc0aec1c1bdc4c1",
            "4c86b25187b26795b35a89b34c94c65294c6cae8c65b96c74da0d852a1d87dbbd85ba2d84eabe853ace858ace85cade8",
        ],
    ),
    (
        "solarise",
        [
            "bac3b28cc3b271c3b26ac3b2bde7b28fe7b274e7b26ce7b2bbd4b28dd4b272d4b26bd4b2bcbab28ebab273bab26cbab2",
            "f2eadbdbeadba8dbdb96eadbe3f1c0dbf1c05885c097f1c0edd5a8dbd5a898b3a896d5a8e8bb91dbbb91b7bb9197bb91",
            "7cea9d7cea9d7bd99d7cea9d7cd8827cd8827873827cd8827cc6717cc6717aa9717cc6717cb66a7cb66a7cb66a7cb66a",
        ],
    ),
    (
        "stripes",
        [
            "ae908cc36d62db675df1675da07e5dc07e5dda7e5df07e5da1915dc1915ddb915df1915da0a25dc0a25ddba25df1a25d",
            "3f82b43f82b45390b44083b53f8fc93f8fc9cae6c8adadb9489edb84b0d0e3b185f29561aebea3c4a769dba25df1a25d",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "scramble",
        [
            "a36662c56663db665df2665ea27e5fc07e5ddc7e5df27e61a2915dc0915ada915bf19061a0a15cc0a25ddaa15cf1a25e",
            "8470757278919f7a858d768970858a7a8894d5c09ba9858f7a948e84958f99aaa9a9949369a3b493a39268a3c99ba3a2",
            "3f82b43f82b45490b53f82b53f8ec8408fc9c8e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4ec3fa4eb3fa4ec",
        ],
    ),
    (
        "digital_glitch",
        [
            "a0675fc0675dda675ef0675d9c805bbe7e5dd97f5fee805ea29361c0935ed9935ded945f9ea25abea25ddaa25deea35c",
            "3f82b44684b4538fb34084b3668b9b748d9acbbf9d838a9e839489979887a99489b4958590a374aca277c2a374d1a573",
            "3f82b44486b25891b33f84b2408fc84795c6c0e5c73f8fc9449bd73f9adb6eb5d6439dd643a5e743a2ea40a5ea44a3e9",
        ],
    ),
    (
        "tape_damage",
        [
            "996861b66c65ce6f68e2716b9b7f65b8826acf836ce3866f9e916aba936ed19571e59774aaab7ec5ad82d9ad81ecaf84",
            "7b758d8f788fa58191ab788f868ea1988fa2d3c3aeb491a38a9cad9b9daeb6acb1b49dae85a1b296a2b2a5a3b3b2a4b4",
            "628db4618cb3749bb7628db4518dbe538dbecde7ce538dbe5799cf5698cf7fb5d45798cf5ba3de5aa3de5ba3de5aa2de",
        ],
    ),
    (
        "flicker_cut",
        [
            "965f56b45f56cc5f56e15f56957656b37656cc7656e07656968756b48756cc8756e18756959756b39756cc9756e19756",
            "3774a13774a14980a13774a1377fb4377fb4b1cfb4377fb43789c43789c45fa2c43789c43793d23793d23793d23793d2",
            "3c7cad3c7cad4f8aad3c7cad3c89c13c89c1bedfc13c89c13c94d23c94d266afd23c94d23c9de23c9de23c9de23c9de2",
        ],
    ),
    (
        "quake",
        [
            "a87a61c47a61dc7a60f17a60a78c5ec48c5edc8c5ff18c5fa89c61c49c61dc9c60f09c60a7a85dc4a85ddca85df1a85e",
            "89708f9d748fad758fb6738c887e99ab929abf999bb9819e898da6a99ea6c2aaa7b890a7879ab09c9db1b0a1b2b99db4",
            "3f8ac23f8ac17cb0c178adc13f95d43f95d38fc3d368aed33f9fe33f9fe359abe341a0e33fa7ef3fa7ef3fa7ef3fa7ef",
        ],
    ),
    (
        "glass_shards",
        [
            "a56579b4655ecf6a5dea71688a7d4ab57d5fd27d5ff7866f9f916eb5915ed29261f99964a5a361b9a364d6a563f8a962",
            "7d759c8675949e7b95a9829f68869d8686a3b3a2a381a1c87996b48696b0a6a4b16ba4df9ba48a8da5b994a8c765adf0",
            "4082b44082b44c8ab7508ebd408ec8408ec88ec2c863a9d2409adb409adb69b4dc49a4e549a7e944a7ec46a8ee43acf6",
        ],
    ),
    (
        "rough_edge",
        [
            "a1675dc1675ddb675df1675da07e5dc07e5dda7e5df07e5d63848db48b63db915df1915d3ea0e4919179dba25df1a25d",
            "4a76a197625adb675df1675d4088bcb6795eda7e5df07e5d3f98d85e859dc78760f1915d3fa4eb429bd8b0916ff0a15d",
            "3f82b43e81b35e76935568803f8fc93e8ec8a3a3843e8ec93f9adb3f9adb65add33e91cd3fa4eb3fa4eb3fa4eb3e9ee1",
        ],
    ),
    (
        "kaleidoscope",
        [
            "ea9855db8f4bdb8e4bea9954ed8c65dc8c61dc8d63e9945be9945bdc8d63dc8c61ed8c65ea9954db8e4bdb8f4bea9855",
            "b398afbbaba6bcaea7b297adac9aadcebfabd4c5aab696acb696acd4c5aacebfabac9aadb297adbcaea7bbaba6b398af",
            "3f98d875bbda75bbd93f99d9429cdaccf0d5b9e4d63f9ee23f9ee2b9e4d6ccf0d5429cda3f99d975bbd975bbda3f98d8",
        ],
    ),
    (
        "liquid_warp",
        [
            "ac6563cd655ee96b5af76e57ad7a5bcd805ee98361f7845baa8e5dca965de6965ef7955ea7a05ec5a65de3a55ff5a45d",
            "907e99c4ac9cc39a9eb5778f8c8aa5aea4a6c7aca2bb84a28696ac9999aebaa6a9b68fa685a2b894a2b6a99eb5b19caf",
            "3f85b9589fc9b5decc3f8ec73f92cf3f99d965b1da3f96d53f9fe33fa2e83fa1e73f9de13fa8f13fa8f23fa8f23fa6ee",
        ],
    ),
    (
        "screen_off",
        [
            "444d52444d52444d52444d52a0adb3a1adb3a2adb3a2adb3a0adb3a1adb3a2adb3a2adb3444d52444d52444d52444d52",
            "121517373f43373f431215171a1e20606c73606c731a1e201a1e20606c73606c731a1e20121517373f43373f43121517",
            "444d52444d52444d52444d529cadb39cadb39fadb39cadb39cadb39cadb39dadb39cadb3444d52444d52444d52444d52",
        ],
    ),
    (
        "push",
        [
            "896e7bb9675fd4675fec675f838378b97e5fd47e5fec7e5f879483b9915fd4915fec915f85a285b9a25fd4a25feca25f",
            "5791b44d80af9e6a6ec16762badcc95a96c09b8070c17e626fb6db4f9bd29d9274c191623fa4eb4aa4e19ca276c1a262",
            "3f82b44284b4528fb4597ba53f8fc965a6c9a7d2c9528aac3f9adb479fdb66b2db5798c03fa4eb3fa4eb3fa4eb55a4ca",
        ],
    ),
    (
        "slide",
        [
            "8a6e6ec1675ddb675df1675d8e8380c07e5dda7e5df07e5d8b947dc1915ddb915df1915d8ca286c0a25ddba25df1a25d",
            "5791b45380add06a65f1675dbadcc96696c2d18070f07e5d6fb6db579bd2d0926ef1915d3fa4eb54a4e2d1a273f1a25d",
            "3f82b44284b4528fb4707b9b3f8fc965a6c9a7d2c9738ab33f9adb479fdb66b2db7198bb3fa4eb3fa4eb3fa4eb72a4cb",
        ],
    ),
    (
        "roll",
        [
            "704e4ab56961cd645fd05955716245b5805ed17e5ee97b60777549b79360d39160ea8e606f7445b39e5cd4a15eeb9f5f",
            "3f83b60d1e2a1d1a109e69664b95c9225274554b3ca37e6265add8378cc97d8849aa8f5f43a3e625648f63633db59c60",
            "38739f4785b1518fb72d5f853f8cc577b2c898c9cb2f6d993f98d84da2da5cacdd317aae3fa2e83fa4eb3da0e52c75a8",
        ],
    ),
    (
        "split",
        [
            "a5675eb36a67c96a6cef675ea57e5eb4806ed48b6aee7e5ea5915eb3926dc9946fee915ea5a25eb4a271c7a270eea25e",
            "bc68654981b05f8fb2d76866bc7f674a8ec5cbe5c4d67f66bc92684999d676b6d7d79268bca2694aa4e64aa4e6d6a269",
            "527ea73f82b45390b4547eac548cbe3f8fc9c7e8c9528cba5299cb3f9adb6bb6db5499cd53a4db3fa4eb3fa4eb53a4da",
        ],
    ),
    (
        "stretch",
        [
            "8a6f77bb675cd8675cf0675c8c8875bc7e5ed87e5ef07e5e8a967ebb915cd8915cf0915c85a280bca25dd8a25df0a25d",
            "3f82b44989b4b1675de6675d3f8fc983bcc9b07e5de57e5d3f9adb55a8dbb1915de6915d3fa4eb3fa4ebb0a25de6a25d",
            "3f82b43f82b4528fb4627b9f3f8fc94e99c9afd8c9628aae3f9adb409bdb67b4db6298bb3fa4eb3fa4eb3fa4eb62a4c8",
        ],
    ),
    (
        "film_roll",
        [
            "734a4db6675fd2675fea675f6d5d40b77e5fd27e5fea7e5f726a4ab7915fd2915fea915f6e7443b7a25fd2a25feaa25f",
            "4888b42d4f69604146ae67627cb3c941678569604bae7e6253a5db416d8e6e6954ae91623fa4eb2d61875a603caea262",
            "3f82b44486b44f8db42e5d813f8fc971afc99bcac93068913f9adb4da2db61afdb30709e3fa4eb3fa4eb3fa4eb2e76a7",
        ],
    ),
    (
        "spring_motion",
        [
            "3f82b4508cb44888b480728c3f8fc99fcac97fb5c977858f3f9adb61aedb54a6db7d959d3fa4eb3fa4eb3fa4eb7aa3a3",
            "356e983f82b4538fb44183b43578a93f8fc9aad4c961a4c93582b93f9adb67b3db469edb358ac63fa4eb3fa4eb3fa4eb",
            "3f82b43f82b45390b44181b33f8fc94190c9c6e7c9408ec73f9adb3f9adb6cb6db409ad93fa4eb3fa4eb3fa4eb40a4e9",
        ],
    ),
    (
        "wave",
        [
            "736c94b06061d86659f1675d697c91b17960da7e60f07e5d6f90a3b58e5bda915cf1915d6fa1a5bea160dba25ef1a25d",
            "3f81b34a7dacc96361ee66593f8dc74787bbbf7564ef7d643f99d94592cbb78669eb8f593fa3e9459cdbb09667eca05f",
            "3f82b43f83b55595bba9829d3f8fc94a96c98bc3cda48f953f9adb3f9adb56aadda89aa93fa4eb3fa4eb3fa4eca6a3ad",
        ],
    ),
    (
        "slice",
        [
            "9d6862bd6761d96960d56c6f9e7f5dc17f5eda7e5de28068a1915dc1915ddb925eed9261a0a25dc0a25ddaa25deea25f",
            "977977777a8da27a903f82b5738686c8825fd77b61a67b83999570c09065d6905f8594b189a179bea166bfa37abf9e80",
            "7887a53f82b55692b53f82b55b91b83f8fc9caeac93f8fc97a9fba469ad870b6d94497d43fa4eb40a4ea47a2e27c8c95",
        ],
    ),
    (
        "louver",
        [
            "774d448f4d44a24d44b34d449f7e5dc07e5dda7e5df07e5da1915dc1915ddb915df1915da0a25dc0a25ddba25df1a25d",
            "3f82b43f82b45390b43f82b42e69942e699492ab942e69942825173025173725173c25179e9f5bbe9f5bd79f5bed9f5b",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3ea2e83ea2e83ea2e83ea2e8",
        ],
    ),
    (
        "frame",
        [
            "f0ebedead2cff7d2d4f5ebe9dcd5cdb8785fe1785df5d5cfdcdccdb8965fe1965df5dccff0f0edeae6cff7e6d4f5f0e9",
            "e7ebeed4ebfbd4ebfbe7ebee8484848d8d8d8d8d8d8484848484848d8d8d8d8d8d848484ebe8e9eedbd9f9dbdceee8e7",
            "e9eef1c9dbe8c9dbe8e9eef1c7dae83f8bc37eb4c3c7dae8c7deef3f9de051a8e0c7deefe9f0f6c9e6fbc9e6fbe9f0f6",
        ],
    ),
    (
        "roll_3d",
        [
            "58373a99544bb2544bc15149625238ba7d5fd87d5fe8795c66603fb9925cd7925ce88e5a5459339a874db3874dc1834b",
            "285272326489844d4c8c403c5393b866a0b99b7352cb72564093cc4898cb9e8757cb8856286a98307eb5817c478c693c",
            "326890346b953d71952246623d89c1559cc8a1cec8295d833d95d5429cdc60afdc2965903285be3489c53489c5225a81",
        ],
    ),
    (
        "flip_motion",
        [
            "8e5a55b45f56c45a55b44b42927655bf7d5eda7d5bcb6951958858bf915ddb915ecb7b4d8b8f52b49958c39053b47945",
            "0000001e191f1f26330000000000001c1f262831380000000000001c232a24303d0000000000001d24261e3143000000",
            "356f993b7aa9457ca03268913981b53f8ec8bde1c83981b5398cc63f9adb67b4db398cc7358dca3b9cdf3893d23284be",
        ],
    ),
    (
        "fold_motion",
        [
            "3f82b43f82b4db675df1675d3f8fc93f8fc9da7e5df07e5d3f9adb3f9adbdb915df1915d3fa4eb3fa4ebdba25df1a25d",
            "3f82b43f82b4db675df1675d3f8fc93f8fc9da7e5df07e5d3f9adb3f9adbdb915df1915d3fa4eb3fa4ebdba25df1a25d",
            "3f82b43f82b4d46861ef675c3f8fc93f8fc9d78160ef7e5d3f9adb3f9adbd49162ef915d3fa4eb3fa4ebd4a162efa15d",
        ],
    ),
    (
        "spin_motion",
        [
            "5b4d36b66e63813c381307077e774abd8661d8755cdb5c5299965ec1985adf8b61c5654f0e0e087e663ae39c5c985b39",
            "000000000000000000000000000000050607050606000000000000070404040406000000000000000000000000000000",
            "050a0e284f6d5c99bb27587c3976a44c92c1a8d7d0327db23272a13f96d548a4e23998d92760883ea1e6276794050e14",
        ],
    ),
    (
        "spin_3d",
        [
            "50313287483f97434364292269553dbd7c5fdb7c5a984d3d6c6440bc925cdc925d975b394d502e887543956e40644426",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "1a35492a5778305e801f3f5827587b3f8dc6aad3c62b61882760893f9adc5eaedb2b6a971a44622a70a02d77aa1f5175",
        ],
    ),
    (
        "spinback_3d",
        [
            "3d2527713c35883c3c5b251f5f4c37bd7b5fdc7b5a974c3d5c5737bc935ddd935d90573733351f685a337b5a354f351d",
            "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            "142a3a234662234662142a3a2553753f8cc5a0ccc52553752760893f9bdc5bacdc276089183f5a266590266590183f5a",
        ],
    ),
    (
        "page_peel",
        [
            "a1675dc1675ddb675de9645ca07e5dc07e5dda7e5dde795aa1915dc1915dd9905cd28760a0a25dc0a25dd79f5cb48f6b",
            "a1675dbd645cca5f575f637ca07e5db87957c0735138719da0905cb68958988270327eb49d9f5ca3925f52839e338ac6",
            "d7b8af758fa64579983773a0b6a58a3c78a4aac8ad3780b57d8979327eb45d9fbf388cc747839c3389c63691d13997d8",
        ],
    ),
    (
        "mirror_transition",
        [
            "9e6861c1675ddb675df1675d9d7f68c07e5dda7e5df07e5d9e9266c1915ddb915df1915d9da26ac0a25ddba25df1a25d",
            "3f82b43f82b4ab7489f0675e3f8fc93f8fc9ad869bef7f603f9adb3f9adbac95a2f092603fa4eb3fa4ebaca3aeefa261",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "block_motion",
        [
            "a1675dc1675dda675ee06764a07e5dc07e5dd87e5ec97e5fa1915dc1915dd6915ace9164a0a25dc0a25dd8a25deaa260",
            "7d738b5a8bb1587eab3f82b4948467b2c2af9cc6c43f8fc98e938187abac6cb4d93f9ada64a3bc67a3ba46a4e541a4e9",
            "3f82b43f82b45390b43f82b44e97c93f8fc9c7e8c93f8fc9459ddb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "whip",
        [
            "976d79be6763d86763ed675f92827abe7e63d87e63ef7e65959382be9163d89163ee916193a286bea263d8a263eea263",
            "558fb45780ac9c6b75bd675bb9dcc96b9bbc998175c27e676bb2db5b9dce9b937dbf915f3fa4eb50a4dc9aa27fc0a263",
            "3f82b44485b45590b4587eaa3f8fc966a4c9b3d6c95f93b83f9adb4ba0db6cb3db589acb3fa4eb3fa4eb3fa4eb53a4d8",
        ],
    ),
    (
        "motion_camera",
        [
            "a56b5ec26a5dda6a60ef6a61a68061c2805fd97f5cee7f5ea5915ec1915cda9160ef9161a6a062c3a061d9a05deea05e",
            "837b96947b96bc9a97b37e988489a29689a2d4c3a1ba95a18294aa9394aabaacadb496ad869fb5979fb6a29fb3b09fb3",
            "3f84b73f84b76ba0b83f84b83f8fca3f8fcacfeeca4b96ca3f99da3f99da77bcda409ada3fa3e93fa3e93fa3e93fa3e9",
        ],
    ),
    (
        "travel_motion",
        [
            "1f2a3a4525255425243113114d7fa0b0795fda795e8140334c91b4b0965eda965e814f321e3b4b454025544024322211",
            "0e1b260e1b262614172f1415609fc2609ec1a67763d7776249a3e04aa2dfa69763d797620e25350e25352625182f2515",
            "0d1a2418314318314331242a214a685197c474aec47f756e215376439fdf4fa7df7f8e740d223118415d18415d313b2f",
        ],
    ),
    (
        "motion_tween",
        [
            "9e6966bc6966d56966eb69669c7f69bb7f69d98769ea7f699d926dbc926dd6946dea926d9da270bca270d5a270eaa270",
            "7e7691937691a97b91b476917f87a09387a0ceb89fb3879f7e96ad9396adb2a4adb496ad7fa3b993a3b9a4a3b9b4a3b9",
            "4b80b05080b0668db05980b04b8ec4508ec4c8e3c4588ec44b99d65099d67ab4d65999d64ba4e650a4e654a4e658a4e6",
        ],
    ),
    (
        "pull_motion",
        [
            "2c1a216d3c39813c368536313e3623b37b60d77b61da7057414029b3945fd6945eda8655282b1a6d643a826439855a33",
            "1f405819344842282e5e31319dc6c63372a07a6449bc7b635cacde337eb47d784ebc94621f53771943603f42295e5231",
            "224662264e6c274e6c10222f3980b460a2c685bbc61b3d57398dc847a0dd55a9dd1b4460225b83266591266591102c3f",
        ],
    ),
    (
        "pop_motion",
        [
            "9c635bc0665ddb665eea635a9a7a5a898291ae9094e97a5b9b8d5b8a99aba19da6ea8d5a9b9d5ac0a25edba25eea9d5a",
            "31648b38739f41799f31648b377caf3f8ec8b8dec8377caf3787c03f9adc65b2dc3787c03180b83893d23893d23180b8",
            "3f82b53f82b55893b53f82b53f8fc93f8fc9cbebc93f8fc93f9adb3f9adb6fb8db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
    (
        "shape_dissolve",
        [
            "a1675dc1675ddb675df1675da07e5db2806dd98462f07e5da1915d5298ce96a6a6f1915d9fa25f41a4ea6aa4c9f1a25d",
            "8a6c774580b26b8ca6e56964508aaa3f8fc9c7e8c99d869d419ad93f9adb6bb6db7097b73fa4eb3fa4eb3fa4eb6da4c9",
            "3f82b43f82b45390b46b7b993f8fc93f8fc9c7e8c96e8ab23f9adb3f9adb6bb6db5a99c83fa4eb3fa4eb3fa4eb50a4e0",
        ],
    ),
    (
        "shape_flow",
        [
            "a1675dc1675ddb675df1675d94806dc07e5dda7e5df07e5d9f9262c1915ddb915df1915da0a25dc0a25ddba25df1a25d",
            "78728e3f82b490858df1675d568aa43f8fc9d2e2c0f07e5d5c97b73f9adb81b5cdf1915d77a3923fa4eb86a3b0f1a25d",
            "3f82b43f82b45390b43f82b43f8fc93f8fc9c7e8c93f8fc93f9adb3f9adb6bb6db3f9adb3fa4eb3fa4eb3fa4eb3fa4eb",
        ],
    ),
];

/// Kinds whose `ORACLE` row is not recorded yet; `print_oracle_rows` is the only row writer.
/// Whoever adds a kind without a row lists it here until the suite has been run once.
const ORACLE_PENDING: &[&str] = &[];

/// Card-engine kinds: their cards may leave an uncovered or antialiased edge mid-transition
/// (a flip is edge-on at its midpoint), so they are pinned by their oracle row and the
/// endpoint contract instead of the per-tick opacity check.
const CARD_KINDS: &[&str] = &[
    "push",
    "slide",
    "roll",
    "split",
    "stretch",
    "film_roll",
    "spring_motion",
    "wave",
    "slice",
    "louver",
    "frame",
    "roll_3d",
    "flip_motion",
    "fold_motion",
    "spin_motion",
    "spin_3d",
    "spinback_3d",
    "page_peel",
    "mirror_transition",
    "block_motion",
    "whip",
    "motion_camera",
    "travel_motion",
    "motion_tween",
    "pull_motion",
    "pop_motion",
    "shape_dissolve",
    "shape_flow",
];

/// The 4x4 mean-RGB fingerprint at the midpoint of the red-to-blue scene, for the kinds whose
/// midpoint varies across the frame (the uniform kinds record an exact pixel in `midpoint`).
const MIDPOINTS: &[(&str, &str)] = &[
    (
        "light_fade",
        "ff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ff",
    ),
    (
        "film_fade",
        "800080800080800080800080800080800080800080800080800080800080800080800080800080800080800080800080",
    ),
    (
        "soft_focus_fade",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "burn_through",
        "c055ffff9ec1f16cffff460cff8efffd0000ff7d1df42a05f50700ff5b13ff9be7ff7affff811e0300ffff9e7de868ff",
    ),
    (
        "brightness_fade",
        "c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5c200b5",
    ),
    (
        "pixel_blocks",
        "0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff",
    ),
    (
        "straight_wipe",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "soft_edge_wipe",
        "ba00bdbb00bcbc00bbbd00baba00bdbb00bcbc00bbbd00baba00bdbb00bcbc00bbbd00baba00bdbb00bcbc00bbbd00ba",
    ),
    (
        "clock_wipe",
        "ff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000200ff0000ffff0000e8007a8300e40200ff",
    ),
    (
        "corner_sweep",
        "bc00bc0000ff0000ff0000ffff0000bc00bc0000ff0000ffff0000ff0000bc00bc2400fdff0000ff0000fd0024bc00bc",
    ),
    (
        "star_wipe",
        "ff0000a000d3a000d3ff00000000ff0000ff0000ff0000ffff00000000ff0000ffff0000d000a39100dc9100dcd000a3",
    ),
    (
        "panel_wipe",
        "0000ff0000ff0000ffff00000000ff0000fff3005bff00000000ff5b00f3ff0000ff00000000ffff0000ff0000ff0000",
    ),
    (
        "stair_wipe",
        "7000ecb000c6dd008ff900435b00f3a000d3d300a0f3005b5b00f3a000d3d300a0f3005b7000ecb000c6dd008ff90043",
    ),
    (
        "stretch_wipe",
        "0000ff0000ff0000ff0100ff0000ff0000ff0000ff0100ff0000ff0000ff0000ff0100ff0000ff0000ff0000ff0100ff",
    ),
    (
        "neon_edge_wipe",
        "48ffff4dffffffffffffffff48ffff4dffffffffffffffff48ffff4dffffffffffffffff48ffff4dffffffffffffffff",
    ),
    (
        "circle_reveal",
        "ff0000df008bdf008bff0000df008b0000ff0000ffdf008bdf008b0000ff0000ffdf008bff0000df008bdf008bff0000",
    ),
    (
        "box_reveal",
        "ff0000ff0000ff0000ff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000ff0000ff0000ff0000",
    ),
    (
        "diamond_reveal",
        "ff0000bc00bcbc00bcff0000bc00bc0000ff0000ffbc00bcbc00bc0000ff0000ffbc00bcff0000bc00bcbc00bcff0000",
    ),
    (
        "cross_reveal",
        "ff00000000ff0000ffff00000000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ffff00000000ff0000ffff0000",
    ),
    (
        "double_doors",
        "ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000",
    ),
    (
        "corner_inset",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000",
    ),
    (
        "zoom_cross",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "zoom_smear",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "spin_smear",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "streak_smear",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "lens_defocus",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "bloom",
        "ff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ffff00ff",
    ),
    (
        "flash",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    ),
    (
        "lens_flare",
        "c330bed050c1d050c1c330bee979c9f083cbf083cbd050c1d050c1f083cbf083cbe979c9c330bed050c1d050c1c330be",
    ),
    (
        "light_leak",
        "ffbae8fd87d2cd42c1bf14bcffd4f0ff9ed6dc52c2c21fbcffafdbff88ccdf4ec0c422bcff76c5f463c1d63fbdc31cbc",
    ),
    (
        "colour_leak",
        "e061caf894cdfbadd5e2ade9fb72dbffa4ddffc1e5edc9ffff66e6ff86e5fc92dcdc8ddef54ddff457dce051cfca3fc4",
    ),
    (
        "colour_burn",
        "f639fff639fff639fff639fff639fff639fff639fff639fff639fff639fff639fff639fff639fff639fff639fff639ff",
    ),
    (
        "light_sweep",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    ),
    (
        "light_rays",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "phosphor_trail",
        "c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0c055c0",
    ),
    (
        "solarise",
        "8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b8b008b",
    ),
    (
        "stripes",
        "0000ff0000ff0000ff0000ff0000ff0000ff8900e0bb00bc6a00eea300d1f70049ff0000ed006dff0000ff0000ff0000",
    ),
    (
        "scramble",
        "0000ff0000ff0000ffff0000ff0000ff0000ff0000ff0000ff00000000ffff00000000ffff0000ff00000000ffff0000",
    ),
    (
        "digital_glitch",
        "0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ffff0000ff0000ff0000ff0000",
    ),
    (
        "tape_damage",
        "b34cb3a925a9a200a2a400a4a000a0b95cb9aa29aab143b1a716a7b34cb3b247b2b550b5b95bb9b143b1b042b0a818a8",
    ),
    (
        "flicker_cut",
        "0000e40000e40000e40000e40000e40000e40000e40000e40000e40000e40000e40000e40000e40000e40000e40000e4",
    ),
    (
        "quake",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "glass_shards",
        "b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2b900c2",
    ),
    (
        "rough_edge",
        "3824143824143824142e1d963824143824143824142113cf38241438241435225c0000ff3824143421650f07f10000ff",
    ),
    (
        "kaleidoscope",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "liquid_warp",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "screen_off",
        "b6cbd6b9cfdab9cfdab6cbd6b9cfdabfd6e1bfd6e1b9cfdab9cfdabfd6e1bfd6e1b9cfdab6cbd6b9cfdab9cfdab6cbd6",
    ),
    (
        "push",
        "0000ff3000fcf2005eff00000000ff3000fcf2005eff00000000ff3000fcf2005eff00000000ff3000fcf2005eff0000",
    ),
    (
        "slide",
        "0000ff3000fcf2005eff00000000ff3000fcf2005eff00000000ff3000fcf2005eff00000000ff3000fcf2005eff0000",
    ),
    (
        "roll",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "split",
        "fe00144200f94200f9fe0014fe00144200f94200f9fe0014fe00144200f94200f9fe0014fe00144200f94200f9fe0014",
    ),
    (
        "stretch",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "film_roll",
        "bcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbc080808080808080808080808080808080808080808080808",
    ),
    (
        "spring_motion",
        "0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff",
    ),
    (
        "wave",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "slice",
        "0000ffd5009c7100eb0000ff0000ffff0000d5009cff0000ff0000ff0000ff00000000ffd5009cff0000ff0000bc00bc",
    ),
    (
        "louver",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "frame",
        "ffffffffffffffffffffffff000000000000000000000000000000000000000000000000ffffffffffffffffffffffff",
    ),
    (
        "roll_3d",
        "0000ec0000ecec0000ec00000000ec0000ecec0000ec00000000ec0000ecec0000ec00000000ec0000ecec0000ec0000",
    ),
    (
        "flip_motion",
        "0000000000000000c40000000000000000000000c40000000000000000000000c40000000000000000000000c4000000",
    ),
    (
        "fold_motion",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "spin_motion",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "spin_3d",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "spinback_3d",
        "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    ),
    (
        "page_peel",
        "eb0000ea00000000c40000c4ea0000ea00000000c40000c40000c40000c40000c40000c40000c40000c40000c40000c4",
    ),
    (
        "mirror_transition",
        "0000ff0000ffc600b1ff00000000ff0000ffc600b1ff00000000ff0000ffc600b1ff00000000ff0000ffc600b1ff0000",
    ),
    (
        "block_motion",
        "ff00000000ffbc00bc0000ff0000ff0000ff8900e10000ffff0000ff00000000ff0000ffff00000000ff0000ff0000ff",
    ),
    (
        "whip",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "motion_camera",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "travel_motion",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "motion_tween",
        "bc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bcbc00bc",
    ),
    (
        "pull_motion",
        "0000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff00000000ff0000ffff0000ff0000",
    ),
    (
        "pop_motion",
        "5000f65000f65000f65000f65000f60000ff0000ff5000f65000f60000ff0000ff5000f65000f65000f65000f65000f6",
    ),
    (
        "shape_dissolve",
        "ff0000ff0007eb0072be00b9ff0007eb0073bc00bb7500eaeb0072bc00bb7300eb0800ffbe00b97500ea0800ff0000ff",
    ),
    (
        "shape_flow",
        "0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0000ff0300ff0000ff0000ff0000ff3d00fa",
    ),
];

/// Kinds whose `MIDPOINTS` row is not recorded yet, for the same reason as `ORACLE_PENDING`.
const MIDPOINTS_PENDING: &[&str] = ORACLE_PENDING;

fn scene(kind: &str) -> Project {
    let mut d = Project::empty(ProjectId(0), "transition goldens".into())
        .unwrap()
        .into_data();
    d.canvas.width = 4;
    d.canvas.height = 4;
    d.scene_mut().timeline.main.items = [(10, 0, [255, 0, 0, 255]), (11, 100, [0, 0, 255, 255])]
        .into_iter()
        .map(|(id, start, c)| Item {
            hidden: false,
            retime: Default::default(),
            id: ItemId(id),
            source_asset: None,
            source_range: SourceRange::new(Tick(100), Tick(200)).unwrap(),
            start: Tick(start),
            content: ItemParams::Overlay(OverlayParams { color: Color(c) }),
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
        })
        .collect();
    d.scene_mut().timeline.transitions.push(Transition {
        from_pieces: vec![],
        to_pieces: vec![],
        id: TransitionId(1),
        track: TrackId(0),
        from: ItemId(10),
        to: ItemId(11),
        kind: kind.into(),
        start: Tick(80),
        duration: Tick(40),
        alignment: TransitionAlignment::Center,
        params: Default::default(),
    });
    Project::new(d).unwrap()
}

/// The exact midpoint frame between pure red and pure blue, in linear light, where the frame is
/// one uniform colour; `None` when the kind varies across the frame, whose midpoint fingerprint
/// is recorded in `MIDPOINTS`.
fn midpoint(kind: &str) -> Option<[f32; 4]> {
    let linear = |v: u8| {
        let c = f32::from(v) / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    match kind {
        "cross_dissolve" => Some([0.5, 0., 0.5, 1.]),
        "dip_to_black" => Some([0., 0., 0., 1.]),
        "dip_to_white" => Some([1.; 4]),
        "dip_to_color" => Some([linear(217), linear(51), linear(51), 1.]),
        _ => None,
    }
}

#[test]
fn viewport_hit_test_selects_visible_transition_contributors_in_source_handles() {
    for definition in TRANSITION_DEFINITIONS {
        let project = scene(definition.kind);
        let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
        let viewport = render::viewport::Viewport::new(
            Rect {
                x: 0.,
                y: 0.,
                width: 4.,
                height: 4.,
            },
            [4, 4],
            1.,
            [0., 0.],
        )
        .unwrap();
        for at in [79, 80, 81, 99, 100, 101, 119, 120] {
            let expected = if definition.kind.starts_with("dip_") {
                if at < 100 {
                    Some(ItemId(10))
                } else if at == 100 {
                    None
                } else {
                    Some(ItemId(11))
                }
            } else if at <= 80 {
                Some(ItemId(10))
            } else {
                Some(ItemId(11))
            };
            assert_eq!(
                renderer.hit_test(&viewport, [2., 2.], Tick(at)).unwrap(),
                expected,
                "{} at {at}",
                definition.kind
            );
            if let Some(id) = expected {
                assert!(renderer.item_geometry(id, Tick(at)).unwrap().is_some());
            }
        }
    }
}

#[test]
fn every_catalog_kind_has_start_mid_end_goldens_without_flash_or_frame_drop() {
    for def in TRANSITION_DEFINITIONS {
        let p = scene(def.kind);
        let h = Hooks::default();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        for (at, expected) in [
            (79, [1., 0., 0., 1.]),
            (80, [1., 0., 0., 1.]),
            (120, [0., 0., 1., 1.]),
            (121, [0., 0., 1., 1.]),
        ] {
            let im = r.render_frame(Tick(at), &mut Empty, &h).unwrap();
            assert!(
                im.pixels()
                    .iter()
                    .all(|p| p.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6)),
                "{} at {at}",
                def.kind
            );
        }
        let im = r.render_frame(Tick(100), &mut Empty, &h).unwrap();
        match midpoint(def.kind) {
            Some(expected) => assert!(
                im.pixels()
                    .iter()
                    .all(|p| p.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6)),
                "{} at 100",
                def.kind
            ),
            None => match MIDPOINTS.iter().find(|(kind, _)| *kind == def.kind) {
                Some(recorded) => assert_eq!(&fingerprint(&im), recorded.1, "{} at 100", def.kind),
                None => assert!(
                    MIDPOINTS_PENDING.contains(&def.kind),
                    "{}: add its midpoint fingerprint",
                    def.kind
                ),
            },
        }
        // Every tick around and across the half-open span yields finite working pixels. Every
        // pre-existing kind stays opaque; a new card kind may leave an uncovered or antialiased
        // edge (a flip is edge-on at its midpoint), so those are pinned by their oracle row and
        // the endpoint contract instead. Adjacent-frame smoothness is meaningful only for a
        // per-pixel mix.
        let uncovered = CARD_KINDS.contains(&def.kind);
        let mut previous = None;
        for at in 79..=121 {
            let im = r.render_frame(Tick(at), &mut Empty, &h).unwrap();
            assert!(
                im.pixels()
                    .iter()
                    .all(|p| p.iter().all(|c| c.is_finite() && (0. ..=1.).contains(c)))
            );
            if !uncovered {
                assert!(im.pixels().iter().all(|p| p[3] == 1.));
            }
            if let Some(prev) = previous
                && midpoint(def.kind).is_some()
            {
                assert!(
                    im.pixels()[0]
                        .iter()
                        .zip(prev)
                        .all(|(a, b): (&f32, f32)| (a - b).abs() <= 0.051),
                    "{} at {at} jumps by more than 0.051",
                    def.kind
                );
            }
            previous = Some(im.pixels()[0]);
        }
    }
}

#[test]
fn item_effects_masks_opacity_and_muted_hidden_tracks_compose_before_transition() {
    let p = scene("cross_dissolve");
    let mut d = p.into_data();
    for item in &mut d.scene_mut().timeline.main.items {
        item.effects.push(
            effect_definition("blur")
                .unwrap()
                .instance("blur")
                .with("intensity", EffectValue::Scalar(10.))
                .unwrap(),
        );
        item.masks
            .push(Mask::new("rectangle", MaskShape::Rectangle));
        item.animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::new(ParamValue::Scalar(0.5)).unwrap(),
            )
            .unwrap();
    }
    d.scene_mut().timeline.main.muted = true;
    let p = Project::new(d).unwrap();
    let h = Hooks::default();
    let im = Renderer::new(&p, RenderSettings::from_project(&p))
        .unwrap()
        .render_frame(Tick(100), &mut Empty, &h)
        .unwrap();
    // Compare the midpoint with independent evaluations of each already-filtered
    // side over the same backdrop, through the ordinary item compositor.
    let mut sides = Vec::new();
    for id in [10, 11] {
        let mut d = p.clone().into_data();
        d.scene_mut().timeline.transitions.clear();
        d.scene_mut()
            .timeline
            .main
            .items
            .retain(|i| i.id == ItemId(id));
        let side = &mut d.scene_mut().timeline.main.items[0];
        side.start = Tick(0);
        let side = Project::new(d).unwrap();
        sides.push(
            Renderer::new(&side, RenderSettings::from_project(&side))
                .unwrap()
                .render_frame(Tick(50), &mut Empty, &h)
                .unwrap(),
        );
    }
    for ((actual, a), b) in im
        .pixels()
        .iter()
        .zip(sides[0].pixels())
        .zip(sides[1].pixels())
    {
        assert!(
            actual
                .iter()
                .zip(a.iter().zip(b))
                .all(|(d, (a, b))| (d - (a + b) * 0.5).abs() < 1e-6)
        );
    }
    let mut d = p.into_data();
    d.scene_mut().timeline.main.hidden = true;
    let hidden = Project::new(d).unwrap();
    let im = Renderer::new(&hidden, RenderSettings::from_project(&hidden))
        .unwrap()
        .render_frame(Tick(100), &mut Empty, &h)
        .unwrap();
    assert!(im.pixels().iter().all(|p| *p == [0., 0., 0., 1.]));
}

struct Cancel {
    checks: AtomicUsize,
    cancel_at: usize,
    inner: Hooks,
}
impl ExecutionHooks for Cancel {
    fn cancelled(&self) -> bool {
        self.checks.fetch_add(1, Ordering::SeqCst) >= self.cancel_at
    }
    fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
        self.inner.reserve(kind, bytes)
    }
}
#[test]
fn cancellation_between_sides_drops_all_leases_and_next_frame_succeeds() {
    let p = scene("cross_dissolve");
    let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let baseline = Cancel {
        checks: AtomicUsize::new(0),
        cancel_at: usize::MAX,
        inner: Hooks::default(),
    };
    drop(r.render_frame(Tick(100), &mut Empty, &baseline).unwrap());
    let checkpoints = baseline.checks.load(Ordering::SeqCst);
    for cancel_at in 0..checkpoints {
        let h = Cancel {
            checks: AtomicUsize::new(0),
            cancel_at,
            inner: Hooks::default(),
        };
        assert!(matches!(
            r.render_frame(Tick(100), &mut Empty, &h),
            Err(Error::Cancelled)
        ));
        assert_eq!(h.inner.used.load(Ordering::SeqCst), 0);
    }
    let h = Hooks::default();
    drop(r.render_frame(Tick(100), &mut Empty, &h).unwrap());
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
}

#[test]
fn transition_consumes_both_video_handles_at_exact_source_ticks() {
    struct Sources {
        calls: Vec<(ItemId, Tick)>,
    }
    impl SourceProvider for Sources {
        fn status(
            &mut self,
            _: &media::SourceAsset,
            _: bool,
            _: &dyn ExecutionHooks,
        ) -> Result<Option<UnavailableReason>> {
            Ok(None)
        }
        fn video(
            &mut self,
            _: &media::SourceAsset,
            id: ItemId,
            at: Tick,
            h: &dyn ExecutionHooks,
        ) -> Result<Option<Image>> {
            self.calls.push((id, at));
            Ok(Some(Image::new(
                4,
                4,
                if id == ItemId(10) {
                    [1., 0., 0., 1.]
                } else {
                    [0., 0., 1., 1.]
                },
                h,
            )?))
        }
        fn audio(
            &mut self,
            _: &media::SourceAsset,
            _: ItemId,
            _: i64,
            n: usize,
            rate: u32,
            h: &dyn ExecutionHooks,
        ) -> Result<AudioBlock> {
            AudioBlock::new(rate, 2, n, MemoryKind::Decoded, h)
        }
    }
    let mut d = scene("cross_dissolve").into_data();
    d.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("transition-pixels"))
            .format("synthetic")
            .duration(Tick(1000))
            .stream(
                testkit::source::video()
                    .codec("rgba")
                    .time_base(media::TimeBase::new(1, 30).unwrap())
                    .duration(Tick(1000))
                    .size(4, 4)
                    .build(),
            )
            .build(),
    );
    for i in &mut d.scene_mut().timeline.main.items {
        i.source_asset = Some(media::AssetId(1));
        i.content = ItemParams::Video(VideoParams {
            source_audio_enabled: false,
            volume: 0.,
            muted: false,
            fades: Default::default(),
            crop: None,
        });
    }
    let p = Project::new(d).unwrap();
    let h = Hooks::default();
    let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let mut sources = Sources { calls: vec![] };
    for (at, calls) in [
        (80, vec![(ItemId(10), Tick(180)), (ItemId(11), Tick(80))]),
        (100, vec![(ItemId(10), Tick(200)), (ItemId(11), Tick(100))]),
        (120, vec![(ItemId(11), Tick(120))]),
    ] {
        sources.calls.clear();
        drop(r.render_frame(Tick(at), &mut sources, &h).unwrap());
        assert_eq!(sources.calls, calls);
    }
    for hidden_side in [10, 11] {
        let mut d = p.clone().into_data();
        d.scene_mut()
            .timeline
            .main
            .items
            .iter_mut()
            .find(|i| i.id == ItemId(hidden_side))
            .unwrap()
            .animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::new(ParamValue::Scalar(0.)).unwrap(),
            )
            .unwrap();
        let p = Project::new(d).unwrap();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        sources.calls.clear();
        let frame = r.render_frame(Tick(100), &mut sources, &h).unwrap();
        assert_eq!(sources.calls.len(), 1);
        assert_ne!(sources.calls[0].0, ItemId(hidden_side));
        let expected = if hidden_side == 10 {
            [0., 0., 0.5, 1.]
        } else {
            [0.5, 0., 0., 1.]
        };
        assert!(frame.pixels().iter().all(|p| *p == expected));
    }
}

fn frame(width: u32, height: u32, outgoing: bool) -> Image {
    let pixels = (0..width * height).map(|n| {
        let (x, y) = (
            (n % width) as f32 / width as f32,
            (n / width) as f32 / height as f32,
        );
        if outgoing {
            [0.2 + 0.7 * x, 0.1 + 0.3 * y, 0.05, 1.]
        } else {
            [0.05, 0.2 + 0.2 * y, 0.4 + 0.5 * x, 1.]
        }
    });
    Image::from_working_pixels(width, height, pixels, &Hooks::default()).unwrap()
}
/// The default parameters plus each parameter alone at its bounds or every choice.
fn variants(definition: &TransitionDefinition) -> Vec<TransitionParams> {
    let mut all = vec![TransitionParams::new()];
    for param in definition.params {
        let values = match param.kind {
            TransitionParamKind::Float { min, max, .. } => {
                vec![TransitionValue::Float(min), TransitionValue::Float(max)]
            }
            TransitionParamKind::Choice { options, .. } => (0..options.len() as u32)
                .map(TransitionValue::Choice)
                .collect(),
            TransitionParamKind::Bool { .. } => {
                vec![TransitionValue::Bool(false), TransitionValue::Bool(true)]
            }
            TransitionParamKind::Color { .. } => vec![
                TransitionValue::Color(Color([0; 4])),
                TransitionValue::Color(Color([255; 4])),
            ],
            TransitionParamKind::Point { .. } => vec![
                TransitionValue::Point([0., 0.]),
                TransitionValue::Point([1., 1.]),
            ],
            TransitionParamKind::Angle { .. } => vec![
                TransitionValue::Angle(-1440.),
                TransitionValue::Angle(1440.),
            ],
            TransitionParamKind::Seed { .. } => {
                vec![TransitionValue::Seed(0), TransitionValue::Seed(9999)]
            }
        };
        for value in values {
            all.push(TransitionParams::from([(param.id.to_owned(), value)]));
        }
    }
    all
}
fn mean_abs(a: &Image, b: &Image) -> f32 {
    a.pixels()
        .iter()
        .zip(b.pixels())
        .flat_map(|(x, y)| x.iter().zip(y).map(|(x, y)| (x - y).abs()))
        .sum::<f32>()
        / (a.pixels().len() * 4) as f32
}

#[test]
fn every_registered_kind_meets_the_endpoint_finite_deterministic_continuous_contract() {
    let mut registered: Vec<_> = transitions::registered_kinds().collect();
    let mut catalogue: Vec<_> = TRANSITION_DEFINITIONS.iter().map(|d| d.kind).collect();
    registered.sort_unstable();
    catalogue.sort_unstable();
    assert_eq!(
        registered, catalogue,
        "catalogue and dispatcher list one set"
    );
    let h = Hooks::default();
    for definition in TRANSITION_DEFINITIONS {
        for params in variants(definition) {
            let case = format!("{} {params:?}", definition.kind);
            for (w, ht) in [(16, 9), (1, 1), (1, 7), (7, 1), (2, 2)] {
                let (a, b) = (frame(w, ht, true), frame(w, ht, false));
                let run = |p: f64, scale: f32| {
                    transitions::apply(definition.kind, &params, &a, &b, p, scale, &h)
                        .unwrap_or_else(|error| panic!("{case}: {error:?}"))
                };
                assert_eq!(run(0., 1.).pixels(), a.pixels(), "{case} p=0 is outgoing");
                assert_eq!(run(1., 1.).pixels(), b.pixels(), "{case} p=1 is incoming");
                assert_eq!(run(-3., 1.).pixels(), a.pixels(), "{case} clamps below");
                assert_eq!(run(3., 1.).pixels(), b.pixels(), "{case} clamps above");
                for scale in [1., 0.5, 4.] {
                    for p in [0.001, 0.25, 0.5, 0.75, 0.999] {
                        let image = run(p, scale);
                        assert_eq!((image.width(), image.height()), (w, ht));
                        assert!(
                            image.pixels().iter().all(|px| px
                                .iter()
                                .all(|c| c.is_finite() && (-1e-6..=1. + 1e-6).contains(c))
                                && px[..3].iter().all(|c| *c <= px[3] + 1e-6)),
                            "{case} {w}x{ht} p={p} scale={scale}"
                        );
                        assert_eq!(
                            image.pixels(),
                            run(p, scale).pixels(),
                            "{case} deterministic"
                        );
                    }
                }
                assert!(
                    mean_abs(&run(0.001, 1.), &a) < 0.02,
                    "{case} continuous at 0"
                );
                assert!(
                    mean_abs(&run(0.999, 1.), &b) < 0.02,
                    "{case} continuous at 1"
                );
            }
        }
        // High-frequency premultiplied frames exercise boundaries that an opaque
        // ramp and mean-only comparison cannot establish.
        for alpha in [0., 0.25, 1.] {
            let a = detailed_frame(64, 36, alpha, false);
            let b = detailed_frame(64, 36, alpha, true);
            for (p, source) in [(0.00001, &a), (0.99999, &b)] {
                let image = transitions::apply(
                    definition.kind,
                    &TransitionParams::new(),
                    &a,
                    &b,
                    p,
                    1.,
                    &h,
                )
                .unwrap();
                assert!(
                    max_abs(&image, source) < 0.002,
                    "{} alpha={alpha} endpoint={p}",
                    definition.kind
                );
                assert!(
                    image
                        .pixels()
                        .iter()
                        .all(|px| px[..3].iter().all(|v| *v <= px[3])
                            && px.iter().all(|v| v.is_finite() && (0. ..=1.).contains(v)))
                );
            }
        }
    }
    for kind in ["quake", "screen_off"] {
        for (width, height) in [(1, 1), (64, 36)] {
            for alpha in [0., 0.25, 1.] {
                let a = detailed_frame(width, height, alpha, false);
                let b = detailed_frame(width, height, alpha, true);
                let params = if kind == "quake" {
                    TransitionParams::from([
                        ("shake".into(), TransitionValue::Float(400.)),
                        ("motion_blur".into(), TransitionValue::Float(100.)),
                    ])
                } else {
                    TransitionParams::new()
                };
                for end in [false, true] {
                    let mut previous = f32::INFINITY;
                    for distance in [0.001, 0.0001, 0.00001] {
                        let p = if end { 1. - distance } else { distance };
                        let image = transitions::apply(kind, &params, &a, &b, p, 1., &h).unwrap();
                        let error = max_abs(&image, if end { &b } else { &a });
                        assert!(
                            error <= previous + 1e-6,
                            "{kind} converges at {p}: {error} > {previous}"
                        );
                        previous = error;
                    }
                    assert!(
                        previous < 0.0002,
                        "{kind} alpha={alpha}: limiting error {previous}"
                    );
                }
            }
        }
    }
}

fn detailed_frame(width: u32, height: u32, alpha: f32, incoming: bool) -> Image {
    let pixels = (0..width * height).map(|n| {
        let on = ((n % width) + n / width).is_multiple_of(2) != incoming;
        if on {
            [alpha, 0., alpha * 0.25, alpha]
        } else {
            [0., alpha * 0.5, alpha, alpha]
        }
    });
    Image::from_working_pixels(width, height, pixels, &Hooks::default()).unwrap()
}
fn max_abs(a: &Image, b: &Image) -> f32 {
    a.pixels()
        .iter()
        .zip(b.pixels())
        .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
        .fold(0., f32::max)
}

#[test]
fn field_wipes_match_spatial_and_scaled_geometry() {
    let h = Hooks::default();
    let red = [1., 0., 0., 1.];
    let blue = [0., 0., 1., 1.];
    let run = |kind: &str, params: &TransitionParams, w, ht, p, scale| {
        transitions::apply(
            kind,
            params,
            &Image::new(w, ht, red, &h).unwrap(),
            &Image::new(w, ht, blue, &h).unwrap(),
            p,
            scale,
            &h,
        )
        .unwrap()
    };
    for (angle, incoming, outgoing) in [
        (90., (10, 20), (100, 20)),
        (0., (60, 70), (60, 10)),
        (180., (60, 10), (60, 70)),
        (270., (110, 20), (10, 20)),
    ] {
        let params = TransitionParams::from([("angle".into(), TransitionValue::Angle(angle))]);
        let image = run("straight_wipe", &params, 120, 80, 0.25, 1.);
        assert_eq!(
            image.pixels()[incoming.1 * 120 + incoming.0],
            blue,
            "incoming from angle {angle}"
        );
        assert_eq!(image.pixels()[outgoing.1 * 120 + outgoing.0], red);
    }
    for kind in [
        "circle_reveal",
        "box_reveal",
        "diamond_reveal",
        "cross_reveal",
    ] {
        let image = run(kind, &TransitionParams::new(), 120, 80, 0.5, 1.);
        assert_eq!(
            image.pixels()[40 * 120 + 60],
            blue,
            "{kind} reveals its centre"
        );
        assert_eq!(
            image.pixels()[0],
            red,
            "{kind} retains the corners at midpoint"
        );
    }
    let params = TransitionParams::from([
        ("feather".into(), TransitionValue::Float(24.)),
        ("antialias".into(), TransitionValue::Choice(0)),
    ]);
    let large = run("straight_wipe", &params, 128, 72, 0.5, 1.);
    let small = run("straight_wipe", &params, 64, 36, 0.5, 0.5);
    for y in 0..36 {
        for x in 0..64 {
            for channel in 0..4 {
                let expected = (large.pixels()[(y * 2) * 128 + x * 2][channel]
                    + large.pixels()[(y * 2) * 128 + x * 2 + 1][channel])
                    * 0.5;
                assert!(
                    (small.pixels()[y * 64 + x][channel] - expected).abs() < 0.005,
                    "scaled feather at {x},{y}"
                );
            }
        }
    }
}

#[test]
fn gaussian_transitions_preserve_axes_and_bound_scratch_work() {
    let h = Hooks::default();
    for kind in ["soft_focus_fade", "bloom"] {
        let param = if kind == "bloom" { "radius" } else { "blur" };
        for (width, height, scale) in [(1920, 1, 1.), (1, 8192, 1024.)] {
            let pixels: Vec<_> = (0..width * height)
                .map(|n| {
                    if n < width * height / 2 {
                        [0., 0., 0., 1.]
                    } else {
                        [1.; 4]
                    }
                })
                .collect();
            let a = Image::from_working_pixels(width, height, pixels.iter().copied(), &h).unwrap();
            let transposed =
                Image::from_working_pixels(height, width, pixels.iter().copied(), &h).unwrap();
            let params = TransitionParams::from([(param.into(), TransitionValue::Float(400.))]);
            let run = |image: &Image| {
                transitions::apply(kind, &params, image, image, 0.5, scale, &h).unwrap()
            };
            let (result, rotated) = (run(&a), run(&transposed));
            assert!(
                mean_abs(&result, &rotated) < 2e-5,
                "{kind}: transpose preserves blur strength"
            );
            assert!(result.pixels().iter().flatten().all(|v| v.is_finite()));
            if kind == "soft_focus_fade" && width == 1920 {
                // Independent step-edge geometry: sigma is about 185 px, so
                // a sample 100 px before the edge must receive substantial light.
                assert!(result.pixels()[860][0] > 0.15);
                assert!(result.pixels()[1060][0] < 0.85);
                assert!(
                    result
                        .pixels()
                        .windows(2)
                        .all(|px| px[0][0] <= px[1][0] + 1e-5)
                );
            }
        }
    }
    let (a, b) = (frame(8192, 1, true), frame(8192, 1, false));
    let bytes = Image::bytes(8192, 1).unwrap();
    // Destination + two blur images fit exactly; column scratch must be admitted too.
    let budget = Hooks {
        limit: bytes * 3,
        ..Hooks::default()
    };
    assert!(
        matches!(transitions::apply("soft_focus_fade", &TransitionParams::new(), &a, &b, 0.5, 1., &budget), Err(Error::Budget { requested, .. }) if requested == bytes)
    );
    assert_eq!(budget.peak.load(Ordering::SeqCst), bytes * 3);
    assert_eq!(budget.used.load(Ordering::SeqCst), 0);

    struct ScratchCancel {
        inner: Hooks,
        reservations: AtomicUsize,
        checks: AtomicUsize,
        cancel_at: usize,
    }
    impl ExecutionHooks for ScratchCancel {
        fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
            self.reservations.fetch_add(1, Ordering::SeqCst);
            self.inner.reserve(kind, bytes)
        }
        fn cancelled(&self) -> bool {
            self.reservations.load(Ordering::SeqCst) >= 4
                && self.checks.fetch_add(1, Ordering::SeqCst) >= self.cancel_at
        }
    }
    for cancel_at in [0, 2, 12] {
        let hooks = ScratchCancel {
            inner: Hooks::default(),
            reservations: AtomicUsize::new(0),
            checks: AtomicUsize::new(0),
            cancel_at,
        };
        assert!(matches!(
            transitions::apply(
                "soft_focus_fade",
                &TransitionParams::new(),
                &a,
                &b,
                0.5,
                1.,
                &hooks
            ),
            Err(Error::Cancelled)
        ));
        assert_eq!(
            hooks.reservations.load(Ordering::SeqCst),
            4,
            "cancel inside admitted scratch initialization"
        );
        assert_eq!(hooks.inner.peak.load(Ordering::SeqCst), bytes * 4);
        assert_eq!(hooks.inner.used.load(Ordering::SeqCst), 0);
    }
    assert!(matches!(
        transitions::apply(
            "soft_focus_fade",
            &TransitionParams::new(),
            &a,
            &b,
            0.5,
            f32::MAX,
            &h
        ),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn dispatcher_refuses_hostile_inputs_and_cancels_without_leaking() {
    let h = Hooks::default();
    let (a, b) = (frame(8, 8, true), frame(8, 8, false));
    let none = TransitionParams::new();
    for definition in TRANSITION_DEFINITIONS
        .iter()
        .filter(|d| d.param("seed").is_some())
    {
        let bad = TransitionParams::from([("seed".into(), TransitionValue::Seed(u32::MAX))]);
        for p in [0., 0.5, 1.] {
            assert!(matches!(
                transitions::apply(definition.kind, &bad, &a, &b, p, 1., &h),
                Err(Error::Invalid(_))
            ));
            assert!(
                render::transition_thumbnail::ThumbnailCache::new(1)
                    .thumbnail(definition.kind, &bad, (8, 8), p as f32, &h)
                    .is_err()
            );
        }
    }
    let wrong_type = TransitionParams::from([("hold".to_owned(), TransitionValue::Bool(true))]);
    let unknown = TransitionParams::from([("nope".to_owned(), TransitionValue::Bool(true))]);
    let short = frame(8, 4, false);
    for (kind, params, other, p, scale) in [
        ("not_a_kind", &none, &b, 0.5, 1.),
        ("cross_dissolve", &none, &short, 0.5, 1.),
        ("cross_dissolve", &none, &b, f64::NAN, 1.),
        ("cross_dissolve", &none, &b, f64::INFINITY, 1.),
        ("cross_dissolve", &none, &b, 0.5, 0.),
        ("cross_dissolve", &none, &b, 0.5, f32::NAN),
        ("cross_dissolve", &none, &b, 0.5, -1.),
        ("dip_to_color", &wrong_type, &b, 0.5, 1.),
    ] {
        assert!(
            transitions::apply(kind, params, &a, other, p, scale, &h).is_err(),
            "{kind} {p} {scale}"
        );
    }
    // A stray override never changes a kind that does not read it.
    transitions::apply("cross_dissolve", &unknown, &a, &b, 0.5, 1., &h).unwrap();
    for kind in transitions::registered_kinds() {
        for cancel_at in [0, 1, 5] {
            let hooks = Cancel {
                checks: AtomicUsize::new(0),
                cancel_at,
                inner: Hooks::default(),
            };
            let (a, b) = (frame(64, 64, true), frame(64, 64, false));
            let result = transitions::apply(kind, &none, &a, &b, 0.5, 1., &hooks);
            assert!(matches!(result, Err(Error::Cancelled)), "{kind}");
            assert_eq!(hooks.inner.used.load(Ordering::SeqCst), 0);
        }
    }
    for (kind, p, sides) in [
        ("cross_dissolve", 0., (true, false)),
        ("cross_dissolve", 0.5, (true, true)),
        ("dip_to_black", 0.49, (true, false)),
        ("dip_to_white", 0.5, (false, false)),
        ("dip_to_color", 0.51, (false, true)),
    ] {
        assert_eq!(
            transitions::exposed_sides(kind, &none, p).unwrap(),
            sides,
            "{kind} {p}"
        );
    }
    let hold = TransitionParams::from([("hold".to_owned(), TransitionValue::Float(50.))]);
    assert_eq!(
        transitions::exposed_sides("dip_to_color", &hold, 0.4).unwrap(),
        (false, false)
    );
}

#[test]
fn thumbnails_are_generated_bounded_shared_and_evicted() {
    let h = Hooks::default();
    let cache = transition_thumbnail::ThumbnailCache::new(2);
    let none = TransitionParams::new();
    let first = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert_eq!((first.width(), first.height()), (32, 18));
    let again = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "same key shares one image"
    );
    let other = cache
        .thumbnail("dip_to_black", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert_ne!(first.pixels(), other.pixels());
    let start = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0., &h)
        .unwrap();
    let end = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 1., &h)
        .unwrap();
    assert_ne!(start.pixels(), end.pixels(), "the two frames differ");
    // Capacity two: the oldest entry was dropped, so the same key renders a new image.
    let rerun = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &rerun));
    assert_eq!(first.pixels(), rerun.pixels());
    for size in [(0, 9), (9, 0), (513, 9), (9, 513)] {
        assert!(
            cache
                .thumbnail("cross_dissolve", &none, size, 0.5, &h)
                .is_err()
        );
    }
    for p in [f32::NAN, f32::INFINITY] {
        assert!(
            cache
                .thumbnail("cross_dissolve", &none, (4, 4), p, &h)
                .is_err()
        );
    }
    assert!(
        cache
            .thumbnail("not_a_kind", &none, (4, 4), 0.5, &h)
            .is_err()
    );
    drop((first, again, other, start, end, rerun));
    drop(cache);
    assert_eq!(
        h.used.load(Ordering::SeqCst),
        0,
        "no lease outlives the cache"
    );
}

/// A 4x4 grid of mean 8-bit RGB per cell, as 96 hex characters: one oracle row per progress.
fn fingerprint(image: &Image) -> String {
    let h = Hooks::default();
    let bytes = image.rgba8(13, &h).unwrap();
    let (width, height) = (image.width() as usize, image.height() as usize);
    let mut sums = [[[0u64; 3]; 4]; 4];
    let mut counts = [[0u64; 4]; 4];
    for (n, px) in bytes.bytes.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (n % width, n / width);
        let (column, row) = (x * 4 / width, y * 4 / height);
        counts[row][column] += 1;
        for (channel, value) in sums[row][column].iter_mut().zip(px) {
            *channel += u64::from(*value);
        }
    }
    let mut out = String::with_capacity(96);
    for row in 0..4 {
        for column in 0..4 {
            let count = counts[row][column].max(1);
            for channel in &sums[row][column] {
                out.push_str(&format!("{:02x}", (channel / count).min(255) as u8));
            }
        }
    }
    out
}

/// The report's two procedural frames, premultiplied and opaque.
fn oracle_frames(width: u32, height: u32) -> (Image, Image) {
    let frame = |outgoing: bool| {
        let pixels = (0..width * height).map(|n| {
            let x = (n % width) as f32 + 0.5;
            let y = (n / width) as f32 + 0.5;
            let (w, h) = (width as f32, height as f32);
            if outgoing {
                let cell = (w / 8.).max(1.);
                let checker = if ((x / cell).floor() + (y / cell).floor()) % 2. == 0. {
                    0.15
                } else {
                    0.
                };
                [
                    (0.2 + 0.7 * x / w + checker).min(1.),
                    0.1 + 0.3 * y / h,
                    0.05 + checker,
                    1.,
                ]
            } else {
                let disc = if (x - w * 0.62).hypot(y - h * 0.4) < h * 0.22 {
                    0.6
                } else {
                    0.
                };
                [
                    (0.05f32 + disc).min(1.),
                    (0.2 + 0.2 * y / h + disc).min(1.),
                    0.4 + 0.5 * y / h,
                    1.,
                ]
            }
        });
        Image::from_working_pixels(width, height, pixels, &Hooks::default()).unwrap()
    };
    (frame(true), frame(false))
}

#[test]
fn every_kind_matches_its_recorded_oracle_row_at_three_progress_points() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(128, 72);
    for definition in TRANSITION_DEFINITIONS {
        let Some(row) = ORACLE.iter().find(|(kind, _)| *kind == definition.kind) else {
            assert!(
                ORACLE_PENDING.contains(&definition.kind),
                "{}: add its oracle row",
                definition.kind
            );
            continue;
        };
        for (progress, expected) in [0.25f64, 0.5, 0.75].into_iter().zip(row.1) {
            let image = transitions::apply(
                definition.kind,
                &TransitionParams::new(),
                &a,
                &b,
                progress,
                1.,
                &h,
            )
            .unwrap();
            assert_eq!(
                &fingerprint(&image),
                expected,
                "{} at {progress}",
                definition.kind
            );
        }
    }
    for &kind in ORACLE_PENDING {
        assert!(
            ORACLE.iter().all(|(recorded, _)| *recorded != kind),
            "{kind}: recorded; remove it from ORACLE_PENDING"
        );
        assert!(
            TRANSITION_DEFINITIONS.iter().any(|d| d.kind == kind),
            "{kind}: pending kind is not registered"
        );
    }
    assert!(
        ORACLE_PENDING
            .iter()
            .enumerate()
            .all(|(n, kind)| ORACLE_PENDING.iter().skip(n + 1).all(|other| other != kind))
    );
    assert_eq!(
        ORACLE.len() + ORACLE_PENDING.len(),
        TRANSITION_DEFINITIONS.len()
    );
}

#[test]
#[ignore = "prints the oracle table after an intended change"]
fn print_oracle_rows() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(128, 72);
    for definition in TRANSITION_DEFINITIONS {
        let prints: Vec<_> = [0.25f64, 0.5, 0.75]
            .into_iter()
            .map(|progress| {
                format!(
                    "\"{}\"",
                    fingerprint(
                        &transitions::apply(
                            definition.kind,
                            &TransitionParams::new(),
                            &a,
                            &b,
                            progress,
                            1.,
                            &h,
                        )
                        .unwrap(),
                    )
                )
            })
            .collect();
        println!("    (\"{}\", [{}]),", definition.kind, prints.join(", "));
    }
    for definition in TRANSITION_DEFINITIONS {
        if midpoint(definition.kind).is_some() {
            continue;
        }
        let p = scene(definition.kind);
        let h = Hooks::default();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        let im = r.render_frame(Tick(100), &mut Empty, &h).unwrap();
        println!("    (\"{}\", \"{}\"),", definition.kind, fingerprint(&im));
    }
}

/// A value pair that must move the picture away from the catalogue default.
fn changed_values(param: &TransitionParamDef) -> Vec<TransitionValue> {
    let different = |v: f64, default: f64| (v - default).abs() > 1e-9;
    match param.kind {
        TransitionParamKind::Float {
            min, max, default, ..
        } => [
            default + (max - default) * 0.5,
            default - (default - min) * 0.5,
            max,
            min,
        ]
        .into_iter()
        .filter(|v| different(*v, default))
        .map(TransitionValue::Float)
        .collect(),
        TransitionParamKind::Choice { options, default } => (0..options.len() as u32)
            .filter(|n| *n != default)
            .map(TransitionValue::Choice)
            .collect(),
        TransitionParamKind::Bool { default } => vec![TransitionValue::Bool(!default)],
        TransitionParamKind::Color { .. } => vec![
            TransitionValue::Color(Color([0, 0, 0, 255])),
            TransitionValue::Color(Color([255, 0, 255, 255])),
        ],
        TransitionParamKind::Point { default } => [[0., 0.], [1., 1.]]
            .into_iter()
            .filter(|v| v != &default)
            .map(TransitionValue::Point)
            .collect(),
        TransitionParamKind::Angle { default } => [default + 70., default - 70.]
            .into_iter()
            .map(TransitionValue::Angle)
            .collect(),
        TransitionParamKind::Seed { default } => [default.wrapping_add(1).min(9999), 0, 9999]
            .into_iter()
            .filter(|v| *v != default)
            .map(TransitionValue::Seed)
            .collect(),
    }
}

#[test]
fn every_parameter_changes_the_picture() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(160, 90);
    let progresses = [0.3f64, 0.5, 0.7];
    let render = |kind: &str, params: &TransitionParams| {
        progresses.map(|progress| {
            transitions::apply(kind, params, &a, &b, progress, 1., &h)
                .unwrap_or_else(|error| panic!("{kind} {progress}: {error:?}"))
        })
    };
    for definition in TRANSITION_DEFINITIONS {
        for param in definition.params {
            let mut baseline = TransitionParams::new();
            if param.id == "border_color" {
                baseline.insert("border_width".into(), TransitionValue::Float(12.));
            }
            // The default circle is rotation-invariant; measure every Shape Dissolve parameter
            // on a star so rotation still has a picture to change.
            if definition.kind == "shape_dissolve" {
                baseline.insert("shape".into(), TransitionValue::Choice(3));
            }
            let base = render(definition.kind, &baseline);
            let mut choices = vec![base];
            let mut best = 0f32;
            for value in changed_values(param) {
                if value == param.kind.default_value() {
                    continue;
                }
                let mut params = baseline.clone();
                params.insert(param.id.to_owned(), value.clone());
                if params.get(param.id) == baseline.get(param.id) {
                    continue;
                }
                let frames = render(definition.kind, &params);
                let difference = |other: &[Image; 3]| {
                    frames
                        .iter()
                        .zip(other)
                        .map(|(a, b)| mean_abs(a, b))
                        .fold(0., f32::max)
                };
                best = best.max(difference(&choices[0]));
                if matches!(value, TransitionValue::Choice(_)) {
                    for other in &choices {
                        assert!(
                            difference(other) > 1e-5,
                            "{} {} {value:?} duplicates another choice",
                            definition.kind,
                            param.id
                        );
                    }
                    choices.push(frames);
                }
            }
            assert!(
                best > 1e-5,
                "{} {} never changes the picture ({best})",
                definition.kind,
                param.id
            );
        }
    }
}

/// Prints median wall time per catalogue kind at 1920x1080/30 fps, five reps after one warm-up.
#[test]
#[ignore = "cost report, not a gate"]
fn transition_cost_at_1080p() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(1920, 1080);
    let mut rows: Vec<(f64, &str)> = Vec::new();
    for definition in TRANSITION_DEFINITIONS {
        let run = || {
            transitions::apply(
                definition.kind,
                &TransitionParams::new(),
                &a,
                &b,
                0.5,
                1.,
                &h,
            )
            .unwrap();
        };
        run();
        let mut samples: Vec<f64> = (0..5)
            .map(|_| {
                let start = std::time::Instant::now();
                run();
                start.elapsed().as_secs_f64() * 1000.
            })
            .collect();
        samples.sort_by(|a, b| a.total_cmp(b));
        rows.push((samples[2], definition.kind));
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (ms, kind) in &rows {
        println!("{ms:8.2} ms  {kind}");
    }
}
