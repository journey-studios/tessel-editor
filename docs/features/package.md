# F026 — Empacotar e restaurar projetos (P07)

## Status

Lib **V** (`crates/media/tests/package.rs`, `flow.rs`), `scripts/media` **V**, UI **V** (diálogo Empacotar;
`scripts/live-flow-proof.py` empacota, valida o `.tesselpack` com `zipfile` do Python, torna os originais
indisponíveis e restaura em outra pasta).

## Contrato

Três operações distintas: **salvar** o documento (F022), **empacotar** (documento + todas as mídias que ele usa)
e **exportar vídeo** (F029). Empacotar nunca altera nem apaga os originais.

```text
project.tessel.json        documento com caminhos relativos ao pacote
manifest.json              format "tessel-package" v1: arquivos, tamanhos, SHA-256, completo?, runtime exigido
media/<sha256>/<nome>      uma cópia por conteúdo distinto
media/seq-<asset>/<frame>  sequências de frames (cada frame listado no manifest)
```

Formatos: `archive` (padrão) ou `directory` (`.tesselpack`, ZIP64 próprio sem compressão, adequado a >4 GiB; sem
dependência externa). Extração externa com `unzip`/`zipfile` é parte da prova.

| `op` | Campos | Notas |
|---|---|---|
| `project.package_preflight` | `path`, `format?`, `sequences?`, `allow_partial?`, `replace?` | tamanho estimado, espaço livre, dependências ausentes/alteradas, não portáveis |
| `project.package` | idem + `wait?` | job `package`; copia e verifica **em staging** e publica só ao fim |
| `project.verify` | `path` | confere manifest, caminhos e hashes de um pacote sem extrair para o projeto |
| `project.restore` | `path`, `folder`, `open?` (padrão `true`) | valida caminhos e hashes **antes** de publicar; abre o projeto restaurado |

- `sequences` empacota só essas sequências com o **fechamento transitivo** do que usam (cenas → splats/texturas,
  áudio, texto).
- Mídia ausente ou alterada bloqueia (`missing_media` no preflight); `allow_partial:true` produz um pacote
  marcado `complete:false` no manifest.
- Não sobrescreve destino existente sem `replace:true` (`destination_exists`); sem espaço → blocker no preflight e
  `write_failed` se faltar durante a escrita (staging removido, destino anterior preservado).
- Restauração rejeita entradas com `..`, caminho absoluto, link simbólico ou nomes ambíguos, e limita entradas
  (1.000.000) → `invalid_package`; hash divergente ou arquivo truncado → `integrity_error`.
- Pacote restaurado abre **sem acesso aos originais**; edição/export final exigem dependências resolvidas.

## Evidência

Testes de pacote: round-trip, truncado, hash alterado, caminho malicioso, cancelamento, disco cheio, parcial,
ZIP64: (1) layout com 4,5 GiB para `io::sink` (`entries_past_four_gib_use_64_bit_sizes_and_offsets`, 163 s em debug) e
(2) **em disco real** — `crates/media/tests/zip64_disk.rs` (`#[ignore]`, exige `TESSEL_ZIP64_DISK=1`): confere o espaço livre
com `df -k` (4,5 GiB + 2 GiB de margem; sem espaço, falha com `PENDING (not run)` e não escreve nem apaga nada), grava um
arquivo `big.bin` de 4,5 GiB mais `tail.txt` num `.zip` real, reabre com o `ZipReader` e confirma que o offset da segunda entrada
passa de `u32::MAX`, e então roda **`unzip -tq`** (Info-ZIP, confere todos os CRCs) e **`unzip -p … tail.txt`** (extrai a
entrada que fica depois de 4 GiB); apaga o temporário no fim. Executado em 2026-11-04 (macOS 26.6.2, `df` com ~6,4 GiB livres):
**passou em 204,7 s** (debug). Efeito colateral registrado: com o disco caindo a 2,2 GiB livres, o GC do wrapper `mbx` removeu
o diretório de build gerenciado dele (`target/`, 6,1 GiB, reconstruível); nenhum dado do usuário foi apagado.
Comando: `TESSEL_ZIP64_DISK=1 ./scripts/cargo test -p tessel-editor-media --test zip64_disk --locked -- --ignored --nocapture`.

## Limites

Cópia integral dos originais (sem recorte/handles/transcode). Fontes redistribuíveis não têm política própria
(texto usa a fonte do runtime). Proxies e cache ficam fora do pacote.
