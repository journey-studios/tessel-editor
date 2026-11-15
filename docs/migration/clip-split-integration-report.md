# Integração BL081.2 e BL081.1.1

Branch: feat/clip-split-fades-shortcut. Base main9073e775e0014b3927dfde7ff0a76d8eb4ead2ce,
merge localf217814. Entregas8901553/12ac339 (envelope) e cb86d05/d55e87d (atalho)
preservadas. O único conflito textual foi o backlog: histórico BL081 e o de
claims/P4b foram mantidos; BL082.4.15 usa o estado mais recente da main.

## Resultado e contrato

Cortes dentro de fades preservam quadro/PCM e cada rampa herdada; editar um
fade não renormaliza o outro. Projetos com janela de envelope exigem schema3;
o leitor anterior recusa esse schema. Cmd+B/Ctrl+B chama o mesmo comando no
playhead vivo, sem cortar em campos/diálogos ou em repeat. Entrada de teclado
é N/A no headless/lib: a edição continua clip.split. Contratos: [F017](../features/split.md)
e [F051](../features/audio-fades.md).

## Correção encontrada na integração

A rodada restante do workspace parou com exit101 em scene_frames: o teste
exigia Capability::ALL (14 no Rust), incluindo FxTransition, no anúncio JS.
[F064](../features/fx-transitions.md) mantém S2/runtime e admissão pública
fechados, com13capacidades no JS. A enumeração, o anúncio e o teste original
são idênticos à main9073e77 (comparação de fonte, sem execução isolada da base).

Commit5232ea7 corrige apenas a expectativa do teste: anúncio exato das
capacidades públicas e recusa explícita de fxTransition. Runtime, schemas,
catálogo e regras de admissão não foram ampliados. BL081.16 registra o achado.

## Evidências Mac integradas

| Gate | Resultado |
|---|---|
| fmt, workspace check alltargets, clippy alltargets -Dwarnings, workspace no-run | exit0 |
| Core completo | 1732passam,0falhas,7ignorados existentes |
| Desktop completo | 532passam,0falhas |
| Export FFmpeg | 11/11 |
| Atalho / split / fades GPUI | 16/16,5/5,2/2 |
| Restante do workspace, core/desktop excluídos | exit101 inicial, expectativa de handshake incompatível com F064 |
| Render completo após correção, TESSEL_REQUIRE_RUNTIME=1 | 136passam,0falhas,3ignorados existentes |
| Clippy e build desktop+corebins após correção | exit0 |
| Prova versionada, headless e janela própria | 12/12 e12/12,0skips |

As suítes foram executadas em rodadas separadas; não houve um comando único
cargo test --workspace com exit0. A falha original e a rechecagem estrita
permanecem nos logs. A primeira prova funcional falhou antes dos checks porque
o GC automático do mbx removeu os binários; reconstrução pelo wrapper padrão
e nova prova passaram. Nenhum target-dir alternativo ou clean/gc manual.

OpenQodex inicial b6d7517a2064:40arquivos,118/118trechos e15/15candidatos,
sem achados, antes do ajuste do teste. O ajuste e esta documentação fazem
parte da revisão final pré-push. O recibo final acompanha a descrição do PR.
Logs duráveis no SSD de coordenação, tessel-bl081-integration-20261115:
gates/, workspace-rest.log, render-final.log, recovery-summary.json,
proof-final/, split-proof-final.log e recibos OpenQodex.

## Limites

Windows validou as entregas isoladas anteriores; o merge atual foi executado
no Mac. Sem novo aceite Windows do merge, tecla física, foco na WebView,
áudio ouvido, desempenho ou inspeção de pixels da janela. A prova nativa é
semântica, com WAV offline. Runtime JS completo não foi rodado separadamente.
Publicação em branch/PR é esta integração; merge remoto exige autorização
separada. BL081.7, BL081.1.1.b, BL081.1.2 e os demais gaps mantêm seu status.


## PR68 — ajustes após CodeRabbit

Um comentário menor fora do diff e dois nits foram confirmados e corrigidos:
qualificadores de schema1/2 sem fade_window em project-document, guarda de
pico não nulo para o áudio embutido do vídeo, e comparação WAV antes/depois
do corte nos dois casos do script versionado. Nenhuma regra de produto mudou.
Mac: envelope44/44, prova headless14/14 e janela14/14, fmt e higiene13/13.
As contagens12/12 acima são da prova anterior; as duas verificações extras
por superfície são comparações diretas inteiro/metades, sem remover checks.
Os eventos de tecla, Windows, pixels e áudio ouvido não foram revalidados.


## Atualização da base para c9c5fa0

Avanço da main para c9c5fa081a4b09193885b0c4afa91a1fdcc8ecb0 causou novo
conflito documental. Merge local2455d46 preserva histórico PR69 e BL081;
BL082.4.29 usa o estado C0a mais recente. Nenhum conflito textual de Rust.

Novo snapshot Mac: fmt/checkalltargets/clippy/no-run/builddesktop+corebins0,
core1758/0falhas/7ignorados existentes; desktopFX80, atalho16, split5, fades2
passam. Scene_frames8/8 com TESSEL_REQUIRE_RUNTIME=1; prova semântica
headless14/14 e janela14/14, higiene13/13. Sem nova execução do workspace
inteiro, Windows, demais532testesdesktop ou desempenho nesta atualização;
as contagens anteriores continuam ligadas aos seus snapshots.

Logs: tessel-pr68-main-c9c5fa0 no SSD de coordenação, summary.json e logs de
cada comando. Atualização sujeita ao OpenQodex pré-push; nenhum merge remoto.
