# F026 — Empacotar e restaurar projetos (P07)

## Status

Lib **V** (`crates/media/tests/package.rs`, `package_atomic.rs`, `flow.rs`), `scripts/media` **V**, UI **V** (diálogo Empacotar;
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
- **`replace:true` preserva o pacote anterior em qualquer falha** (BL088, ver abaixo): ele só é apagado depois que o novo
  pacote, completo e verificado, está no destino.
- Restauração rejeita entradas com `..`, caminho absoluto, link simbólico ou nomes ambíguos, e limita entradas
  (1.000.000) → `invalid_package`; hash divergente ou arquivo truncado → `integrity_error`.
- Pacote restaurado abre **sem acesso aos originais**; edição/export final exigem dependências resolvidas.

## Replace atômico (BL088)

Publicação (`package::publish`, depois de `verify_package` do staging e de um último `ctx.check()`):

| Situação do destino | Passos | Se falhar |
|---|---|---|
| não existe | `rename(staging, destino)` | staging removido; nada mais foi tocado |
| arquivo e o novo é arquivo (archive sobre archive) | **um** `rename`, trocado pelo FS num passo | idem: o arquivo anterior nunca saiu do lugar |
| pasta, ou muda o tipo (pasta↔arquivo) | (1) `rename(destino, .tessel-previous-<pid>-<n>)`, (2) `rename(staging, destino)`, (3) apaga o anterior | falha em (1): nada mudou. Falha em (2): o passo (1) é desfeito e o erro mantém o código (`permission_denied`, `disk_full`, `io_error`) com `details.previous_restored:true`. Se o desfazer também falhar: o pacote anterior fica em `.tessel-previous-…` e `details.previous_package` + a mensagem dizem onde |
| existe e `replace:false` | recusa `destination_exists` (também na publicação, contra corrida) | nada mudou |

- Entre (1) e (2) o destino **não existe** (leitor vê "ausente", nunca um pacote pela metade). Isso não é atômico
  contra um escritor que mexe no destino ao mesmo tempo: a proteção é cooperativa, como o lease das raízes.
- Cancelamento é observado até o último `ctx.check()` antes da troca; os poucos `rename` seguintes não são
  interrompidos (completam ou são desfeitos). Cancelar antes preserva o anterior (testado).
- Se o novo pacote foi publicado mas o anterior não pôde ser apagado por inteiro (ex.: subpasta somente leitura), a
  operação **tem sucesso** e `warnings` traz o caminho do resto (`.tessel-previous-…`). O resto é um pacote já
  substituído e pode estar parcial; nada o apaga depois e o prefixo não é o de scratch (`.tessel-staging-`).
- Só remove o que é seu: o staging próprio (`Drop`) e o anterior que acabou de substituir. Pastas vizinhas, incluindo
  nomes `.tessel-staging-*`/`.tessel-previous-*` de terceiros, não são tocadas (o nome do sibling é `pid-n` e nunca reutiliza caminho existente).
- Um crash do processo entre (1) e (2) deixa o anterior em `.tessel-previous-…`: recuperável à mão, não automático.
- `PackageInputs`, `PackageOptions`, `preflight*`, `create`/`create_with`, o formato do resultado e os consumidores de cenas
  não mudaram; `create_with_fs`/`PublishFs` são `#[doc(hidden)]`, só para injetar falha de `rename` em teste.
- Não foi mudado nenhum limite do Windows (`rename` sobre pasta existente, nomes longos): não há teste real lá; BL054.

## Evidência

Replace atômico: `crates/media/tests/package_atomic.rs` (10 testes: falha injetada do `rename` por Directory/Archive e por tipo
de erro, falha do passo de pôr de lado, do desfazer, troca de formato, sucesso sem resíduos, vizinhos intactos, cancelamento
determinístico e em corrida, pai sem permissão, pacote anterior não apagável, e a operação pública `project.package`/`project.verify`
pelo `MediaHost`). Mais evidência em [next-r4-report](../migration/next-r4-report.md).

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
