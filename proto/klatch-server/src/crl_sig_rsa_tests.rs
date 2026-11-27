//! RSA fixtures for `crl_sig` (#1694): a CRL signed with an RSA key was never tested in the repo, because rcgen makes no
//! RSA keys. These are DER bytes made once with the system openssl (OpenSSL 4.1.0), checked in as hex; nothing runs
//! openssl at test time. How they were made, in a throwaway directory:
//!
//! ```text
//! openssl genrsa -out k.pem 2048; openssl genrsa -out other.pem 2048; openssl genrsa -out k1024.pem 1024
//! # a config with a [dn] of CN=sig-ca and basicConstraints CA:true, keyUsage keyCertSign,cRLSign, then
//! openssl req -x509 -new -key k.pem -config ca.cnf -days 36500 -out ca.pem        (and other, k1024 likewise)
//! openssl ca -gencrl -config ca.cnf -keyfile k.pem -cert ca.pem -md sha256 -out crl256.pem   (sha384, sha512 too)
//! openssl crl -in crl256.pem -outform DER -out crl256.der; openssl x509 -in ca.pem -outform DER -out ca.der
//! ```
//!
//! The openssl CA database files `ca ... -gencrl` needs (the config's `database` and `crlnumber`) were made empty in the
//! same directory: `touch index.txt; echo 01 > crlnumber`. The config set `default_crl_days = 3650`: every CRL here (the
//! older fixtures made on 2026-11-27 as well as `CRL_PSS`) has a nextUpdate 3650 days after its thisUpdate, so none is
//! in date past 2036-11 (the certificates `CA`, `CA_1024` and `CA_PSS` were made with `-days 36500`). None of the dates
//! matters to these tests: `judge` reads only the signature and ignores every time in the CRL (expiry is `tls_crl.rs`'s
//! concern, tested with hand-built CRLs in `tls_crl_tests.rs`).
//!
//! `CA_PSS` and `CRL_PSS` (#1704) are a second 2048 bit RSA CA of the same name with a CRL signed with RSA-PSS (SHA-256,
//! salt 32), which `crl_sig` does not know and so leaves to the verifier:
//!
//! ```text
//! openssl ca -gencrl -config ca.cnf -keyfile k.pem -cert ca.pem -md sha256 \
//!     -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:32 -out pss.pem
//! ```
//!
//! `CA` is the 2048 bit certificate, `CRL_*` its CRLs (issuer CN=sig-ca), `CRL_OTHER_KEY` one of the same issuer name
//! signed by another 2048 bit key, and `CA_1024` / `CRL_1024` the same with a 1024 bit key (which the verifier's RSA
//! algorithms refuse as too short).

use super::*;
use crate::der::subject_and_key_of;

fn unhex(h: &str) -> Vec<u8> {
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
        .collect()
}

const CA: &str = "\
308202d5308201bda003020102021474e5bf7c2f72f710a449cfc7a815cc0e26e53663300d06092a864886f70d01010b\
05003011310f300d06035504030c067369672d63613020170d3236313132373033333035345a180f3231323631313033\
3033333035345a3011310f300d06035504030c067369672d636130820122300d06092a864886f70d0101010500038201\
0f003082010a0282010100b827a4f5693ea1d6a6399949c30eae941a8d2c5568e9f2dd7dcee2d5ae4da6cdfee977c6d1\
ae929b8165c15ccb35728281b789c5fb7d0eea4282da9268743e0452225d4385c7fb0fe19c38700fe30ef6590c438004\
07a4c3d8c979d5706739708dc96671d20c1094b0a85cb76d5418b3af96f7afedcef891b33eed832b4f05416391ee5735\
a42396245a9d9a53315e0998d17cbc619cae3654db31c1476d300997651d70941c5dd58f569c522e1fdeb3245d12624b\
4ed7f8cac2a334717c6666e28017fe6d728e0c360b3912c814d83ccae1a7d5c17965f66d5e2de2ea7196bbe15c043f99\
6074ca79c5d7d347550c4a6bfc0b24de57cdd5116653a6bb85ecdb0203010001a3233021300f0603551d130101ff0405\
30030101ff300e0603551d0f0101ff040403020106300d06092a864886f70d01010b050003820101007df8ec6045f372\
09fe087e5b5c634a21ffc705746f40049eebb25040baa02b116350b97152f7568d93c64da26238a63206dfbe02adc653\
477eea167608818bc86c857912b2317e800ffc66bf276ffc75b326809960e3deea406b6539f006597e454b2a82f819f1\
3bc21c70d7a1987ca3f3d0c4d5fc889cf4bec9896f2b4f1bfcea435ce721a6815f4854870fda1009e5d82d360686e768\
b5aab10f043a14ffcdc4e7646de0eb783c8004611282ccc8b38a1e86a3a3e518ceaebafcdba9bf96e5c96770d6c28cf8\
d233f13b0b4d010fc9af0d51b8919cdc4de1eaac5145da9c05fcb36617b50cb00a2113ae7ab5f7a8947faa715a0a528b\
db74c00c28443616fb";

const CA_1024: &str = "\
308201d030820139a00302010202142923cf8b6f503c3d8887e69811a80237ed97ce45300d06092a864886f70d01010b\
05003011310f300d06035504030c067369672d63613020170d3236313132373033333035345a180f3231323631313033\
3033333035345a3011310f300d06035504030c067369672d636130819f300d06092a864886f70d010101050003818d00\
30818902818100c49c5261daeffa2172205f7366f7d2f1ad48f02de180a8f60215bce2560f73ef8a7fe32cd8545711e0\
f65b3a1289f6a0633aeff86ee19f0f352a1b756f4c1f7e5af1501049f8849fd04aeda137d79e81e7f24ba1cbc620abde\
6369eb9f4b8acf849a9ca9f15460aff3a11296739d4324cccd3694e7e766296f34c8762daa07930203010001a3233021\
300f0603551d130101ff040530030101ff300e0603551d0f0101ff040403020106300d06092a864886f70d01010b0500\
03818100619a533d73247cc7d482318a79fde1612887bba740e263ff39c1a2d5700cc04d8de16fb7f2fc8dc6a45ac81a\
93205a43ee569be7eaf681ce6fabfed0210918d646ae9295e3fcfa2b6da35a6415ba82d3f00acb938f9f4aa09f45f9b4\
9d5078a541e55ca701926692f298b5878b31bd4fc51befb107d06a58b2a6db623524abe8";

const CRL_256: &str = "\
308201693053020101300d06092a864886f70d01010b05003011310f300d06035504030c067369672d6361170d323631\
3132373033333130335a170d3336313132343033333130335aa00e300c300a0603551d140403020101300d06092a8648\
86f70d01010b050003820101007af91d5a284ef23b7ab3f1437991cfee066735a92ccf3b6fde409f3d9aab6c2d2e98d7\
253d389c21eb71ad1f23f1faf1307b081b753b1312504cbb67d2dd1e8706c103f809fe7fbd2da3dd20c3fc73a0825387\
eb3fad683895f17cdae13a74e7680792c80481a41c0f2b9096f5536ac147c38fc8a090b1d4c10b848ee99919bcfd0d9b\
b4c56d6537a5556ed5526b3eeb263b2c1818a10434d01afc277bf969d8f61f4269cdf5979021692c4901fa4741c7b223\
03e33a9d781e79b4e12fc911a5b96492bb83607e5dca0cbe0bac9edd9877d7d2564d6d1adc2da284dce3e79d2fb19fa1\
1b6182c55cc79dbb6b0f0d0135bf54153336798f5f6a9e54fcc2846926";

const CRL_384: &str = "\
308201693053020101300d06092a864886f70d01010c05003011310f300d06035504030c067369672d6361170d323631\
3132373033333130335a170d3336313132343033333130335aa00e300c300a0603551d140403020101300d06092a8648\
86f70d01010c0500038201010006dcf9720045005ac31349dc330b8a7430d03be647e10d388cb8b2af76523e0d38dc4f\
4d088e2cd2a0c9b774b68d0561cc2cc93f13bf495adced605b07f3252eeb4e3475b7ec8ebc20b14d6b9486a8729449bf\
90141403c57bfd3cd51e77127cd441b321dbad5a3db3cb26ffe760dfb0b8efc79cb2d34fb1ffd85b1f781a6e74467d0a\
04ce9cf5c5e280fcf3e6d534085c3533b601e99b021808cc0e4c6c02f3d8220aa769b6b731ea8bfcd05643f8f80e8501\
d9e26556c745f1b80b869af923c842414561eb1dcad53cb7d1dcbece12df7f899fda9ae9ea175bbf95ca95649b635ea3\
84b6e5ef7238864d9f1cb903413b456b92835f37294e723b0aea594384";

const CRL_512: &str = "\
308201693053020101300d06092a864886f70d01010d05003011310f300d06035504030c067369672d6361170d323631\
3132373033333130335a170d3336313132343033333130335aa00e300c300a0603551d140403020101300d06092a8648\
86f70d01010d05000382010100ad488787df1d1175ff430cdc4789c1c55a20855cef018ec265a95662958e281dd14a50\
1892d8d6143ddce7fb8a85f4b40f67f17feefee3b2d792d8c9299c9ec5b1c7dead5547ec757be8faf107fcff1459c021\
37e21d36f929f73b0ac3649a3524780939425e171277e190d295158489a3ab826112f8528ad1a2bfcfe8660237d1d7d8\
7d19dc03cb2192968bdb3be6260246c307ac64f3686caaf36da89d867b5df00d8683da9124ffd130a261e71b743d3fe3\
24def6140775f0a0a4b242a10df5b82395a6f262409fc2882624f339cc19a7503d1438490f9850101064fe40bb97bc37\
ad61b57fd40bc1322a0852a632af0a120dfb806ab5eef8eca55f787025";

const CRL_OTHER_KEY: &str = "\
308201693053020101300d06092a864886f70d01010b05003011310f300d06035504030c067369672d6361170d323631\
3132373033333130335a170d3336313132343033333130335aa00e300c300a0603551d140403020101300d06092a8648\
86f70d01010b05000382010100bd0d18e3b4aeda34d82687d7804de0f930065e25fbb650837edc77d64f5a44b26d9625\
61bfe765aa19d1f6ab7ee309d5c5a7f4bf0a920c6af042f5872a44140e8d42534ca4122c3762c51effcd8febac0a214a\
e48d77d716eec912767365a6ab46a4ce5462aaf4c96abb892b473fbc0af44b36789886cb66b6e2f3c5eaa82292a35e33\
7a4500d2bcf9e597ccbc1a16cd9aae51f9a82f07a2760ec61ef3609854ea83810ed4e6bc9ecf247b7a6b5e635c8b0666\
7b7dee9252f7efc8d311fff5eb9ebb98b05c21a2d505df8b71911d2fa691c2b0729cd717f9e436b9100f13853073f6b1\
f94ad6319991a85a3174c8c017c6d12e83ed162ce61e14089c0ab14706";

const CRL_1024: &str = "\
3081e83053020101300d06092a864886f70d01010b05003011310f300d06035504030c067369672d6361170d32363131\
32373033333130335a170d3336313132343033333130335aa00e300c300a0603551d140403020101300d06092a864886\
f70d01010b0500038181004f87e16bb5a4d1bea9d544249b1d9ca90c961d19e9966871aac27d5b2de737ebc2e756fe28\
f913a89d2c1a9a6edd7a2450657d125a7ded67255eb92626c0bd4b11751b0f10990d5dbc4f26e5637e6fc5f8e98633f2\
28c36a5f03d263ec1d8255d047f8f18799bf572d6c9715bcd8afa5873a63140c4232b734819172852e360d";

fn spki(cert: &str) -> Vec<u8> {
    subject_and_key_of(&unhex(cert)).unwrap().1.to_vec()
}

#[test]
fn an_rsa_signature_is_valid_for_sha256_384_and_512_and_only_for_its_key() {
    let key = spki(CA);
    for crl in [CRL_256, CRL_384, CRL_512] {
        assert_eq!(judge(&unhex(crl), &key), Verdict::Valid);
    }
    // the same issuer name, another key
    assert_eq!(judge(&unhex(CRL_OTHER_KEY), &key), Verdict::Invalid);
    // one byte of the signed part, and one byte of the signature
    for at in [60, 365 - 20] {
        let mut bad = unhex(CRL_256);
        bad[at] ^= 0x01;
        assert_ne!(judge(&bad, &key), Verdict::Valid, "byte {at}");
    }
}

/// A 1024 bit RSA key is refused by the verifier's RSA algorithms (2048 to 8192 bits): the verdict is `Invalid`, and the
/// refusal says the key may be the cause, not only the signature (#1694).
#[test]
fn a_too_short_rsa_key_is_refused_and_the_message_says_the_key_may_be_the_cause() {
    let key = spki(CA_1024);
    assert_eq!(judge(&unhex(CRL_1024), &key), Verdict::Invalid);
    let listed = vec![Intermediate {
        subject: subject_and_key_of(&unhex(CA_1024)).unwrap().0.to_vec(),
        spki: key,
    }];
    let crl = CertificateRevocationListDer::from(unhex(CRL_1024));
    let err = check(&[crl], &[], &listed, Path::new("/d/crl.pem")).unwrap_err();
    assert!(
        err.to_string().contains("or its key is not usable"),
        "{err}"
    );
}

const CA_PSS: &str = "\
308202d2308201baa003020102021414dcc8d539524468ad567bce1fdbaed6608f4e2f300d06092a864886f70d01010b05\
003011310f300d06035504030c067369672d63613020170d3236313132373132323532365a180f32313236313130333132\
323532365a3011310f300d06035504030c067369672d636130820122300d06092a864886f70d01010105000382010f0030\
82010a0282010100b34e97550eeccfe97be6bc0b807bf66dd0f169c3c1f8922915f25c1a526ea3017facf7dadee1b9dae5\
0cf6bf7f477dbdd3b9ea7d625aff89d2a7dc75c28ba17efa01c2755c50980fa80cae6f901156a3e98adcc960070f370179\
7f33af287bc40dec577fabeb3bf390be130ccaf4d640b7da360e8799ebf9baeb43d29dab9942e29f3d3f239c14cb97aa95\
52287b1d8c136115fb8dafede8694fc1ef85c8c7cfdfa0bfc9f6bcecd0e7ff535661480524f6347394fba9d87fba73c036\
0cc0e4c00c6d8bb0a935fedc5b54dc034dcb8dad67939a875232a613636e4342261a99bd0e2fc2ca649246f341af146f0e\
1c0c4ed5ef91287f1bf954f78f3e5f64358e510203010001a320301e300f0603551d130101ff040530030101ff300b0603\
551d0f040403020106300d06092a864886f70d01010b0500038201010059e4e80b29f6f9542be14409c6f0b0300b878ad4\
62d07a78fb81699daa28ee0e76f36504933af3e24a829e33bb810e29373ddd32294d70a47bafa0e637510b8981a4e742aa\
a79b4422724d29dcc66bde10bba5d33607d15f4f7b443d29b71716c8cd93c4731aebc90236202c356ad950343e0e9286f1\
a2223a72714eb03e9309c05e783239e6bbcf8cbcf145c8bac5044fbbe7bd086857002c856b649e037e2781f6aeb7cb0c41\
6d587bfc2ddf679b0a934a4b68649d4ed9500acc7b442a632832e003fe4445db95bba675dfb1a92b5ecd540ff64b31c5e4\
d347fa098a51c112a4dbb121fe4f6bb0c510238d22f205094e31eede1eae878a8f9fac4c3ea6e0aa";

const CRL_PSS: &str = "\
308201d2308187020101304106092a864886f70d01010a3034a00f300d06096086480165030402010500a11c301a06092a\
864886f70d010108300d06096086480165030402010500a2030201203011310f300d06035504030c067369672d6361170d\
3236313132373132323532365a170d3336313132343132323532365aa00e300c300a0603551d140403020101304106092a\
864886f70d01010a3034a00f300d06096086480165030402010500a11c301a06092a864886f70d010108300d0609608648\
0165030402010500a2030201200382010100b04bdd8eb7c7ce0c7b4ec313c4cbd788a01f72310a1f2d2fb848ef53d111a0\
45636ea7e9990281d8a72526d6439fc0d15086a524584c2cbba946e159702f313b38ebb9144ed21f0e973fd090c648ac1c\
74fcffa44a3829b9f5e55af5c46480bb992ce2159a40e35f9347b2c5cb8981af71e049a43127968bededfd02d500a1fe1e\
cdd72bb6a90d61f11599912ffd33824b50c5682bf699533718ef1bb9130ac910daf3e53151a401c962dd9ac54456050d9a\
96b86cb2d265973231ac9c75ea529ede5ea84b59dbc35ac01559d04f07635b552be2a37cf028ad7492f4932dab95f0381e\
fe2b780fe3842c85f1bb0f9e48551de6f0fdc89d898a768a652b41265a";

/// #1694 (mutant M8): an RSA-PSS signature is an algorithm `crl_sig` does not know. The verdict is `Unknown`, and it passes
/// through `check` (which refuses only when EVERY listed certificate of the name says `Invalid`), so a start that worked
/// before still works: the verifier at handshake time decides, exactly as before.
#[test]
fn an_rsa_pss_crl_is_unknown_and_passes_through_check() {
    let key = spki(CA_PSS);
    assert_eq!(judge(&unhex(CRL_PSS), &key), Verdict::Unknown);
    let ca_der = unhex(CA_PSS);
    let (subject, _) = subject_and_key_of(&ca_der).unwrap();
    let listed = vec![Intermediate {
        subject: subject.to_vec(),
        spki: key,
    }];
    let crl = CertificateRevocationListDer::from(unhex(CRL_PSS));
    check(
        std::slice::from_ref(&crl),
        &[],
        &listed,
        Path::new("/d/crl.pem"),
    )
    .expect("an Unknown CRL was refused");
    // two listed certificates of the name (a rollover), both Unknown for it: still passes
    let other = spki(CA_1024);
    assert_eq!(judge(&unhex(CRL_PSS), &other), Verdict::Unknown);
    let both = vec![
        Intermediate {
            subject: subject.to_vec(),
            spki: spki(CA_PSS),
        },
        Intermediate {
            subject: subject.to_vec(),
            spki: other,
        },
    ];
    check(&[crl], &[], &both, Path::new("/d/crl.pem"))
        .expect("Unknown next to Unknown was refused");
}
