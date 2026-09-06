# Graph dependency refresh — 2026-09-05

Created-at: 2026-09-05 21:26 +02:00

A Graph és a külön benchmark-workspace ugyanazt a publikált Core-revíziót
használja: `ac1131b5490734b855021f6076753417e1337cd6`. A Parser egyetlen
forrásazonossága `master#284c490d68232973a006bcba974279fed6f89ce8`.
A tesztelt implementáció `ecf08d719c1e6c0dfc70ef7bbbe309e70d3a1719`;
alapja `94efe354`, így mindkét korábbi Graph-helyességi javítást tartalmazza.

A felhasználó kifejezetten kérte a Graph és hasonló csomagok frissítését.
A közös terv a Core `docs/plans/2026-09-05-dependency-refresh/plan.tex` és
lefordított PDF-je; annak fogyasztói bővítése a végrehajtás előtt elkészült.
A változás a két lockra és a root manifestre korlátozódik. A CBOR-csomagok
az új Core függőségei; a benchmark-lock új Graph→Core éle a már meglévő
crate-manifestet követi. P34 algoritmuskód nem került be ebbe a frissítésbe.

## Ellenőrzés

Amaterasu, Rust/Cargo 1.97.1; az ellenőrzött Core SDK-kép
`hymeko-core:1886f17a42d7a3430e4044129f81955615efd583`. A tiszta Linux checkout
ugyanazt a commitot kapta Git bundle-ből. A Windows és a Docker-gazda közötti
fizikai határ indokolja ezt az egyszer használatos ellenőrzési másolatot.
A dependency-fetch ideiglenes SSH-agenttel, szigorú hostkulcs-ellenőrzéssel
történt; az ellenőrzés offline, külön üres target volume-mal futott.

| Kapu | Eredmény |
|---|---|
| `cargo test --workspace --release --locked --offline --no-fail-fast` | 486 PASS, 6 ignored, 0 FAIL |
| `bash scripts/check_governance.sh HEAD` | PASS |
| `cargo bench --manifest-path benchmarks/Cargo.toml --locked --offline --no-run` | mind a 4 benchmark fordult |
| Mindkét workspace locked metadata | pontosan egy Core és egy Parser, azonos források |
| Teszt utáni Git status | tiszta |

A workspace fordítás és teszt 53,76 s, a benchmark fordítás 19,48 s.
A teljes konténer cgroup memória-csúcsa 1 641 177 088 B; a korlát 16 GiB,
swap 0, CPU 6; kilépés 0, OOM false. Ez egy fordítási/tesztelési futás
mérése, nem teljesítmény-összehasonlítás vagy benchmarkkampány.
A hat kihagyott tesztet a teljes [nyers napló](dependency-refresh/tests.log)
azonosítja. A benchmarkok végrehajtási sebességét ez a kapu nem méri.

[Gépi eredmény](dependency-refresh/results.json),
[konténer állapot](dependency-refresh/container-final.json),
[workspace metadata](dependency-refresh/metadata.json),
[benchmark metadata](dependency-refresh/benchmark-metadata.json).
Az első helyi update-nél hibás SSH alias választott hostkulcsot; a sikertelen
napló megmaradt. A sikeres újrafutás ellenőrzött IP/port URL-átírást használt,
a manifest kanonikus URL-je és a hostkulcs-ellenőrzés megmaradt.

Rollback: kizárólag az `ecf08d71` dependency-commit revertje a saját feature
branchen. A korábbi két helyességi javítás és a távoli main megmarad.

## Discovery

```text
DISCOVERY LEDGER — Core-fogyasztók revíziófrissítése
  find: meglévő Graph és Transformation audit-munkafák, Cargo-manifestek és lockfile-ok
  grep: Core-referenciák a root Cargo.toml fájlokban; Graph külön benchmark-lockkal
  symbols: Graph meglévő SYMBOLS.md; Transformationben nincs index
  read: Graph CLAUDE.md, CORE.YAML, governance script; mindkét README és Cargo-manifest
  verdict: EXTEND a meglévő dependency-fájlok a közös, már lefordított dependency-refresh terv szerint
           NEW dátumozott consumer-jelentések és nyers ellenőrzési adatok; nincs új tesztkeretrendszer
```

A `.gitattributes` keresése nem talált meglévő fájlt; az új fájl kizárólag
az ellenőrzési bizonyíték bájtjainak megőrzését írja elő.
