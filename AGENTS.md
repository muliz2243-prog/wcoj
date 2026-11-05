# AGENTS.md — wcoj

Comparador Leapfrog Triejoin (WCOJ) vs Hash Join sobre triángulos dirigidos
`R(a,b) ⋈ S(b,c) ⋈ T(c,a)`. Crate `wcoj`, edition 2024, sin dependencias externas.

## Mapa de módulos

| Archivo | Responsabilidad |
|---|---|
| `src/types.rs` | Solo contratos: `Edge`, `Triangle`, `FlatTrie`, `LevelCursor`, `Query`, `Machine`, `Graph`, `HashInput/Output`, `InputError`, `MemoryError`, `Comparison`, `CliOptions`, `ReportOptions`. Sin lógica. |
| `src/trie.rs` | Construcción CSR de dos niveles (`keys` + `child_offsets`), orden/deduplicación por permutación, cursor monótono, `gallop` + búsqueda binaria. Incluye `build_layout` para aridad K: un `keys` y un `offsets` planos compartidos por todos los niveles, más `level_start`/`offset_start`; el merge sort es iterativo y sin closures. `leapfrog` y sus realineaciones viven aquí porque operan sobre `TrieCursor` y sirven para cualquier K/N. `layout_child_range` recibe la posición absoluta del padre. |
| `src/wcoj.rs` | Query fija (R=[0,1], S=[0,1], T=[1,0]) y máquina de estados con pila explícita. Motor de referencia del CLI de triángulos. Sin recursión. |
| `src/join.rs` | Motor genérico K/N: `build_join` (valida, ordena atributos por variable, construye un `TrieLayout` por relación) y máquina con buffers planos preasignados. `JoinMachine::next_solution` escribe en un `Vec` reutilizado; `JoinIter` implementa `Iterator<Item = Vec<u32>>`. |
| `src/hash_join.rs` | Baseline: índices por columna, estimación de memoria antes de materializar, `R⋈S` y filtro contra T, hasher fijo para reproducibilidad. |
| `src/input.rs` | Lectura, parseo (`u v`, `#` comentarios, líneas vacías), validación, orden/deduplicación, conteo de nodos. |
| `src/report.rs` | `compare` (compara vectores completos) y `format_report` (tiempos enteros en ns y ms, barras, muestra). No altera métricas. |
| `src/main.rs` | Driver y parseo de argumentos; único punto que lee el reloj (`Instant`) y escribe errores a stderr. |

## Comandos de validación

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo run --release -- edges.txt
cargo run --release -- data/dense.txt --sample 3
cargo run --release -- data/dense.txt --limit-mb 1   # debe fallar con exit 1
cargo run --example custom_ternary
cargo bench --bench triangle
cargo bench --bench triangle -- --shape fan --nodes 2000
cargo bench --bench triangle -- --sweep
cargo doc --no-deps
```

`cargo test` ejecuta 46 tests unitarios en `lib`. Los tests van al final del
archivo que prueban y son el único lugar donde se permite `unwrap`/`panic!`.

## Reglas de código (LOTA-F, obligatorias)

- Sin recursión, sin trait objects, sin callbacks/closures en el núcleo, sin
  estado global mutable, sin operadores sobrecargados.
- Núcleo sin `unwrap()`, `expect()`, `panic!()` ni `?`: errores como valores.
  `?` solo en módulos de borde (`main.rs`).
- Tipos explícitos, máximo 3 niveles de indentación, máximo 60 líneas por
  función y 1000 por archivo. Un punto de salida visible por función.
- Nombres y comentarios en inglés; solo comentarios técnicos.

## Decisiones fijas (v1)

- Aridad 2 únicamente; sin generalización a N.
- Triángulos dirigidos contados tal como aparecen; sin reorientar ni deduplicar.
- Los tiempos miden solo la ejecución de cada motor, no la construcción de tries.
- `hash_memory_bytes` cuenta solo tuplas materializadas (12 bytes/tupla): es una
  cota inferior del consumo real de los índices.
- El hash join aborta antes de reservar si la estimación supera `--limit-mb`
  (por defecto 512 MB).
- `HashInput` exige aristas distintas: una repetida multiplica las tuplas
  materializadas. `validate_graph` deduplica en la ruta de archivo; cualquier
  caller en memoria (por ejemplo el bench) debe deduplicar antes.

## Fixtures

- `edges.txt`: dos triángulos acoplados, aristas sueltas, comentarios,
  tabuladores y una arista duplicada. Esperado: 8 aristas, 8 nodos, 6 triángulos.
- `data/triangle.txt`: un ciclo mínimo (3 aristas, 3 triángulos por rotación).
- `data/graph.txt`: 120 nodos, ~2.7k aristas aleatorias deterministas.
- `data/dense.txt`: 400 nodos, ~11.5k aristas; el intermedio `R⋈S` ronda las
  333k tuplas y es el caso donde se mide la ventaja del WCOJ.

## Artefactos publicables

- `README.md` y metadatos en `Cargo.toml` (`description`, `readme`, `license`,
  `keywords`, `categories`). Sin `repository` porque no hay remoto configurado.
- `examples/custom_ternary.rs`: consulta K=3 / N=4 desde memoria, sin CLI.
- `benches/triangle.rs`: harness propio con `Instant` en vez de criterion para
  mantener cero dependencias. Acepta `--nodes`, `--edges`, `--runs`, `--seed`,
  `--limit-mb`, `--shape random|fan` y `--sweep`. La forma `fan` es el peor caso
  del hash join: `N` fuentes y `N` destinos producen `N^2` filas intermedias y
  cero triángulos. Deduplica las aristas antes de comparar. Cargo pasa
  `--bench` al ejecutable: el parser lo ignora.
- `src/lib.rs` documenta la API y trae un doctest ejecutable; `cargo doc
  --no-deps` con `RUSTDOCFLAGS='-D warnings'` debe quedar limpio.
