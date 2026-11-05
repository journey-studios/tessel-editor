# Diagnóstico de disco — somente leitura

Medido em 2026-11-04, 21:47–22:11 (UTC-3). **Nada foi apagado, movido, truncado, desmontado nem encerrado.**
Os únicos processos que parei foram `du`/`find` que eu mesmo lancei, porque competiam por I/O com o benchmark ativo.
Os números são blocos alocados (`du -sk`/`stat %b`). Isso não garante que o APFS devolva esse espaço (hardlinks, clones e arquivos abertos podem retê-lo).
Complementa `docs/disk-investigation-parent-notes.md`, e os achados convergem.

## 1. Estado do disco

| Item | Valor |
|---|---|
| Container APFS | 494,4 GB. Em uso: 492,1 GB (99,5%). Livre: 2,2 GB |
| Volume Data | 460,7 GB consumidos. `df`: 429 GiB usados, 2,1 GiB livres, 100% |
| Volume VM (swap) | 8,6 GB consumidos. `vm.swapusage`: 6,8 de 8 GB em uso, 8 swapfiles de 1 GiB |
| Snapshots locais | Nenhum (`tmutil listlocalsnapshots` vazio em `/` e em Data) |
| Cotas/reservas | Nenhuma exibida em `diskutil apfs list` |
| Arquivos apagados ainda abertos | Nenhum (`lsof +L1` sem resultados) |

Evolução do livre no Data volume (KB):

| Hora | Livre (KB) |
|---|---|
| 21:48:14 | 2.177.392 |
| 21:48:26 | 2.172.908 |
| 21:56:22 | 2.110.036 |
| 21:57:37 | 2.085.660 |
| 22:00:38 | 2.519.500 |
| 22:03:52 | 2.077.784 |
| 22:06:37 | 2.380.344 |
| 22:10:42 | 2.751.748 |
| 22:11:18 | 2.740.844 |

**Leitura:**
- O livre oscila entre 2,0 e 2,8 GB, em ciclos de centenas de MB.
- Não há crescimento descontrolado. Há criação e remoção de temporários por vários agentes, e o disco fica a menos de 3 GB do limite.
- Com 2 GB livres, o swap (já em 6,8 GB) não consegue crescer. Isso é um risco imediato de travamento, mesmo sem nenhum arquivo novo.

Cobertura: o escopo pedido soma ~46 GiB (veja a seção 2). O Data volume usa 429 GiB, então **a maior parte do uso não está nos alvos pedidos**.
- Pelo que medi: simuladores iOS (24,1 GB), `~/www/journeystudios` (69,1 GB, só tamanhos de pasta) e VM (8,6 GB).
- A maior parte do restante não foi varrida: home inteira, `~/Library` e `~/.t3`. O `du` de `~/.t3` e `~/Library` não terminou em tempo razoável e foi interrompido.
- Não dá para afirmar a composição do resto.

## 2. Onde está o espaço nos alvos pedidos

Os tamanhos são alocados e não se somam entre linhas que contêm outras.

| Local | Alocado | Observação |
|---|---|---|
| `/private/tmp` (total) | 23,1 GiB | 1.881 entradas |
| `$TMPDIR` (`/var/folders/h2/6xxl…/T`) | 6,0 GiB | 26.088 entradas |
| `/var/folders/…/X` e `/C` | 2,1 GiB e 1,1 GiB | caches de sistema/apps (Godot 0,33, Chrome helper 0,18, clang 0,15 GiB). Não são candidatos |
| `~/Library/Caches/mbx` | 10,07 GiB | actions 3,44, targets 5,86, incremental 0,74 GiB. `target/` do tessel-editor é symlink para `targets/v1/992a66b6…`, **já incluído** |
| Projeto `tessel-editor` (sem target) | ~10 MB | `runtime/dist` 4 MB, assets 5,4 MB |

Sobre o cache mbx:
- O `du` do dir do target (6,47 GB) é maior que o do subdir `targets` (6,14 GB) porque 2.528 arquivos do target têm hardlinks entre si (315 MB aparentes). Não é espaço extra independente.
- `actions/cas` (3,47 GB) tem 0 de 5.329 arquivos com hardlink. O conteúdo é provavelmente duplicado em parte com `targets`, mas isso é incerteza minha.
- O cache está acima do teto padrão de 8 GiB (10,07 GiB). Isso é compatível com `MBX_GC_MAX_TOTAL_SIZE=64GiB` ou GC adiado, mas **não consegui provar a causa**, porque `~/Library/Caches/mbx/actions/gc` está vazio.
- O target atual foi modificado em 20:24. Não há build mexendo nele neste momento.

## 3. Vídeos desta sessão

Os vídeos finais **não** são o problema. O maior vídeo nas raízes varridas tem 39,8 MB. Total de MP4/WebM > 5 MB: ~0,4 GiB.

- **Técnicos/fixtures de benchmark** (`/tmp/tev/assets/video/*`): `h264-intra-1080` 39,2 MB, `vp9-intra-1080` 39,8 MB, `h264-g24-2160` 31,4 MB e outros (465 MB no dir). Foram gerados em 20:11–20:16 pelo Opus. São ativos da prova em uso.
- **Exports de provas anteriores** (`/tmp/vgp20`, `vgp21`, `scene-api-*`, `scene-effects-*`): 5–17 MB por arquivo, de 1 e 2 de novembro. Pequenos.
- **Export recente do tev**: `/tmp/tev/exp-heavy/baseline-fullhd-gpu-0.mp4` (5,8 MB, 21:48). O diretório `exp-heavy` já tinha sumido minutos depois, então o próprio runner rotaciona esses exports.
- **Frames PNG são muito maiores que os MP4** (veja os candidatos 3 e 4). O diretório `cam/vid` tem ~2,6 MB por frame, 3×360 frames, e nenhum MP4. `vgp22/vid1` e `vid2` seguem o mesmo padrão.

## 4. Processos ativos (preservar)

| PID | O que é | Tocando em |
|---|---|---|
| 19014 (pai 85412) | `runtime/scripts/perf/run.mjs --engine chrome-swiftshader` | `/tmp/perf-raw/drain` (jsonl de ~0,1 a 0,5 MB, desprezível) e `$TMPDIR/puppeteer_dev_chrome_profile-*` |
| 54858 | rodada `chrome-gpu` anterior | já encerrada |
| 47891 | `./target/debug/tessel-editor` (rodando há 1h40) | só caches Metal |
| 49971 e 54943 | tessel-render e ffmpeg citados pelo pai | **não existem mais às 22:11**, nova pasta de staging `…-69442-3` (476 KB) |
| 53021 | `node serve.mjs 8792` (há 2d19h) | cwd `/tmp/vgp22/viewer`, em uso |
| 11435 | `vitest …volume-baseline.spec.ts` (46 s) | `/tmp/tessel-base-426693/node_modules` |

Crescimento: o benchmark ativo escreve poucos MB. **Não há ameaça de disco cheio por um processo isolado.** O risco é o piso já baixo.
O `/tmp/tev` ficou em 1,17 GiB estável nas duas leituras, com rotação própria.

### Vazamento causal: perfis do Puppeteer

- O `run.mjs` abre o Chrome via Puppeteer (`runtime/scripts/perf/chrome.mjs:41`), cada lançamento cria um perfil `puppeteer_dev_chrome_profile-*` (~87 MB) em `$TMPDIR`, e o `browser.close()` da linha 70 normalmente o remove.
- Há 16 perfis, somando 1,08 GiB, o mais antigo de 15:28 e o mais novo de 21:57. 3 estão em uso (`TjLLsx` pelo PID 19039 e filhos, `AXdivx`, `yGpkp6`), com 257 MiB. Os outros **13 não têm processo** e somam ~0,83 GiB.
- Hipótese: rodadas interrompidas ou mortas por agentes não fecharam o browser, então o perfil não foi apagado. Não provei por log.

## 5. Candidatos (todos só leitura; nada autorizado)

Legenda: **Reg.** = regenerável. Datas: nascimento/modificação do diretório raiz.

| # | Caminho | Alocado | Criado / modificado | Tipo e origem | Em uso? | Reg. | Impacto de remover |
|---|---|---|---|---|---|---|---|
| 1 | `/private/tmp/cam/vid/{a,b,c}` (e `cam/` inteiro) | 3,67 GiB (`vid`: 3 × 360 PNGs de ~2,6 MB; `cam` total 3,8 GB) | 01/11 23:48 / 02/11 00:17 | Frames PNG de teste de câmera. O script de origem é `cam/vid/render.mjs`. `a.log` e `b.log` têm 0 byte | Não. `lsof +D` sem resultado | Provavelmente sim (script presente), **incerto** | Perde as comparações a/b/c. Manter `cam/f0.png`, `f179.png`, `f359.png`, `strip.png` (~6 MB) |
| 2 | `/private/tmp/tessel-property-pr` | 4,21 GiB (node_modules 3,73) | 31/10 13:43 / 31/10 18:05 | Worktree git do tesselapp, HEAD destacado em `c48c4466ba` (#772). `git status` limpo | Não. `lsof +D` sem resultado | Sim (checkout + `pnpm install`, que precisa de rede) | Só com `git worktree remove`, **nunca `rm -rf`**, porque o repo mantém metadados. Nada a perder, pois está limpo |
| 3 | `/private/tmp/vgp22/vid1` e `vid2` | 1,20 GiB (875 + 388 MB) | 02/11 | Frames PNG (360 cada) da tarefa VGP22 | Não, mas o `viewer/` vizinho está em uso (PID 53021) | Provavelmente sim, **incerto** | **Não tocar** em `vgp22/viewer` (1,48 GiB, em uso, contém `layer2.bin` de 275 MB) nem em `vgp22/src` (186 MB) |
| 4 | `$TMPDIR/tessel-media-export-disk-*` (16) | 1,10 GiB | 04/11 16:01–19:05 | `small.dmg` de 100 MiB do teste `a_full_disk_ends_the_export_…` em `crates/media/tests/export.rs`. O teste desmonta mas não apaga. Os PIDs do nome estão mortos | Não. Nenhuma das 16 imagens está aberta e nenhum mount aparece | Sim (gerado por teste) | Nenhum. É o candidato mais seguro |
| 5 | `$TMPDIR/puppeteer_dev_chrome_profile-*` **sem processo** (13 de 16) | ~0,83 GiB | 04/11 15:28–21:57 | Perfis Chrome temporários do `run.mjs` | 3 em uso (257 MiB), **excluir** | Sim | Nenhum, desde que o `lsof`/`ps` seja refeito na hora, porque o runner cria perfis novos |

Candidatos secundários (confirmar uso e proveniência antes):

- `/tmp/tessel-home` (597 MB: `.rustup` 463 MB, `.cargo` 56 MB): parece HOME isolado de teste.
- `/tmp/tessel-seek/old/tessel-editor` (130 MB): binário antigo. Preservar os `runs/` e o relatório.
- `/tmp/tvb` (344 MB) e `/tmp/tvb-base` (134 MB): fixtures e evidência do Opus. Só depois de consolidar os resultados.
- Três cópias duplicadas do binário de 130 MB: `/tmp/tessel-bin`, `/tmp/tev/snap/…/bin`, `/tmp/tev/bin-inst`. Duplicam o `target`, mas o `tev` está ativo.
- `node-compile-cache` em `/tmp` (198 MB) e `$TMPDIR` (328 MB): cache regenerável, ganho pequeno.
- `$TMPDIR/tessel-session-*` (499 dirs, 305 MB) e `tessel-proof-*` (200 dirs, 113 MB): resíduo de testes, sem prova de dono, então não os listei como candidatos firmes.

**Excluídos de propósito (não são candidatos):**
- `/tmp/tessel-base-426693` (3,66 GiB): worktree tessel com um `vitest` em execução agora e módulos sharp/libvips abertos por processos Node.
- `/tmp/tev` (1,17 GiB): prova ativa do Opus.
- `/tmp/vgp22/viewer`: em uso pelo PID 53021.
- `~/Library/Caches/mbx` e o `target/` do tessel-editor: cache ativo, sob orçamento de GC.
- O projeto `tessel-editor`, o `docs/` e os assets originais.
- Todas as worktrees `~/.t3/worktrees/tesselapp/*` (28, `scene-preview` sozinho tem 3,9 GB): não analisadas.

## 6. Estimativa conservadora de recuperação

Se o usuário aprovar, sem contar a mesma coisa duas vezes:

| Camada | Itens | Estimativa |
|---|---|---|
| A. Resíduo seguro de teste/runner | #4 e #5 | **~1,9 GiB** (1,10 + 0,83) |
| B. Frames PNG antigos, regeneráveis por script | #1 e #3 | **~4,9 GiB** (3,67 + 1,20) |
| C. Worktree limpa | #2 | **~4,2 GiB** |
| Total A + B + C | | **~11 GiB** |

- Ainda não é possível afirmar o ganho real. Os arquivos são recentes e dificilmente clonados no APFS, mas não verifiquei clones.
- A camada A sozinha dá ~1,9 GiB, o que dobra o espaço livre atual (hoje ~2,7 GiB).
- O cache mbx não entra na conta. Reduzi-lo exige coordenar com os builds.

## 7. Proposta para aprovação do usuário

1. Aprovar a camada A: apagar os 16 `tessel-media-export-disk-*` e os perfis Puppeteer sem processo. Refazer o `lsof`/`ps` no instante da limpeza.
2. Aprovar a camada B se o usuário confirmar que `cam/` e `vgp22/vid1,vid2` não são mais necessários. Manter os PNGs de prova listados.
3. Aprovar a camada C: `git worktree remove /private/tmp/tessel-property-pr` (limpa, sem alterações locais).
4. Fora do escopo do tessel: os simuladores iOS ocupam 24,1 GB (iOS 18.4, 18.3.1 e 26.3.1), mais 7,7 GB em `/Library/Developer/CoreSimulator`. Se algum não for usado, `xcrun simctl runtime delete` é o caminho oficial. Não executado.
5. Não executar `mbx gc`/`clean` nem apagar `target` durante builds. Cache mbx só com orçamento gerenciado.

### Prevenção (não implementada)

- O teste de disco cheio deve apagar o `.dmg` e o diretório por RAII (inclusive em panic) e criar a imagem com 40m, como a versão atual já pede.
- O `perf/chrome.mjs` deve apagar o `userDataDir` em `finally` e passar um diretório próprio, controlado, em vez do padrão do Puppeteer.
- As suites devem limpar suas fixtures ao terminar e ter um orçamento de temporários.
- Fixar o teto do cache mbx por checkout, sem `MBX_GC_MAX_TOTAL_SIZE=64GiB` por comando.
- Separar binários de teste dos diretórios gerenciados pelo GC.

## 8. Incertezas e observações

- O `/tmp/.dispq_*.txt` são arquivos meus (poucos bytes) deixados para trás, porque a regra era não apagar nada. O único outro arquivo criado é este relatório.
- O `ps` expõe o token de um MCP (Axiom) na linha de comando de vários processos. Vale rotacionar esse token e preferir variável de ambiente.
- Os tamanhos mudam entre leituras (o `tev` e o `$TMPDIR` mudam a cada minuto).
- Não verifiquei Spotlight, Time Machine, Lixeira, Docker nem `~/Library` fora de Caches.
