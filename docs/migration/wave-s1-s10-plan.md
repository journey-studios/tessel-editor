# Rodada S1–S10 — PRs independentes

Base funcional: main `c099f10` (PR #4). A reconciliação documental `0074f8e` é preservada no preparo desta rodada. O usuário autorizou dez subthreads Cursor Sonnet 5.5, cada uma em worktree e PR própria, com correção de findings antes de aceite pelo coordenador. Esta instrução substitui a regra histórica de uma PR única para esta rodada.

## Auditoria de encerramento

R1–R10 e o follow-up de cleanup: threads completed, sem activeRunId; task_cancel confirmou completed para encerrar também eventuais filas/watches. Nenhum cargo/rustc/mbx/proof da rodada anterior estava ativo; somente o checkout principal estava registrado por git worktree. Resultados e código mergeados são preservados. Não apagar o app/projeto do usuário, cache compartilhado ou temporários de autoria incerta.

## Frentes e ownership primário

| Frente | Backlog | Entrega e arquivos principais | Integração |
|---|---|---|---|
| S1 | BL094/BL112 | job.discard, proteção de prune e cleanup fora da UI; media/host + scene_resources/cleanup + desktop jobs | Autônoma; harmonizar claims com S2 |
| S2 | BL111 | Claims persistentes entre processos, recuperação de titulares encerrados; scene_resources/claims + lease/cleanup | Autônoma; teste real de dois processos |
| S3 | BL109 | Bytes imutáveis do export, aprovação e estágio por conteúdo; scene_encoder + resource snapshot | Autônoma; não reescrever cena nem fontes |
| S4 | BL102 | Identidade opcional em asset.load/state, capability e recusa de ack antigo; runtime/index/assets + host dirigido | Autônoma; bridge v1 compatível |
| S5 | BL100/BL101 | Attach pesado em job e recursos vazios; media/host + desktop scene_import | Autônoma; harmonizar host com S1 |
| S6 | BL004/BL005 | Propriedades componentes, renderOrder e pose inicial compatível; scene_edit + core queries + inspector | Autônoma; contratos para S8 |
| S7 | BL025 (subconjunto) | Spark na cena dirigida: recurso, plano, preview, pacote e export; core scene_plan + runtime directed-scene/Spark | Sem luz/clima/rig; seam com S3/S9 |
| S8 | BL015/BL042 | Gizmo de translação no canvas, seleção/foco, gesto único e comandos semânticos; scene_interaction + viewport IPC/runtime overlay | Pode usar position vetorial existente; harmonizar S4/S6/S7 |
| S9 | BL016 | Mix offline dirigido com tempo racional e export com áudio; core scene_export + media scene_encoder/audio | Sem dispositivo/TTS; preservar audio:false |
| S10 | BL019 (ampliação) | Oráculo e provas independentes de splat/câmera/áudio/portabilidade/export; scripts e novos testes | Corpus/harness imediato; aceite vivo depois de S7/S9 |

Worktrees permitem edição paralela, mas não tornam merges independentes semanticamente. Cada agente pode alterar os adapters estritamente necessários à própria fatia; não editar a implementação de outra frente. Conflitos de host/cleanup/plano/runtime são resolvidos pelo coordenador, com nova validação após rebase. Não copiar arquivos inteiros do checkout canônico para sobrepor trabalho alheio.

## Contratos compartilhados

- Core puro, sem GPUI/wry; lib, JSONL e UI usam as mesmas regras. Novos controles expõem estado e ação semântica. Preservar comandos, IDs e formatos antigos; mudança aditiva/capability negociada.
- S1: `job.discard {job}` inicia descarte observável em background, retém dono dos arquivos até terminar; recusa job ativo ou resultado já aplicado de forma tipada. Prune não perde resultado pendente. O destrutor da UI não faz hashing pesado.
- S2: claim é por recurso/identidade/holder, persistido sob lease. Um processo não apaga recurso pendente/aceito de outro. Recuperação não usa PID isolado como identidade; provar queda e reuso sem apagar arquivo aceito.
- S3: export congela bytes aprovados além do mapa, fora da UI. Mutações de mesmo tamanho após resolve e durante job são detectadas ou isoladas pela cópia imutável. Cleanup só remove o estágio próprio e funciona com cancelamento/falha.
- S4: echo aditivo da identidade do pedido e capability declarada. Novo host não pode exigir campo de runtime antigo sem negociação. Ack antigo no mesmo slot nunca confirma carga nova quando capability ativa.
- S5: bundle lido/validado off-thread, candidato aplicado ao projeto correto sem duplicar regra no desktop. Manifesto vazio não precisa de diretório; manifesto com arquivos mantém preflight e contenção.
- S6: paths tipados para componentes e renderOrder; batch atômico, reset e no-op definidos. Pose inicial respeita conversão de movimentos/keys da origem, com oráculo fixo; não inventar editor geral de keyframes.
- S7: Spark básico existente é reutilizado. ScenePlan carrega referência aprovada, geração, matriz e câmera avaliadas; runtime só desenha. Preview/headless/export coincidem. Domínios de luz/clima não suportados continuam recusados explicitamente.
- S8: desenho/hit test no canvas, nunca overlay GPUI fingido sobre WKWebView. Eventos de ponteiro viram intent validado no core. Begin/update/commit/cancel preservam uma entrada de undo; comando semântico de translação dá mesmo resultado sem janela. Escape e foco com política explícita.
- S9: frames e samples mapeados por racionais; suporte inicial ao mix WAV declarado no bundle, respeitando intervalo/offset e conversão de rate. Mix real no arquivo exportado, sem declarar som ouvido ou playback em dispositivo. `audio:false` omite com aviso; unsupported preservado para formatos não atendidos.
- S10: esperados independentes da implementação de destino, tolerâncias fixadas antes dos resultados, controles negativos. Fases ausentes saem BLOCKED (exit !=0); PR draft enquanto o aceite dependente não existir. Não substituir pixel/áudio por schema/ffprobe apenas.

## PR e revisão obrigatórios

Uma PR por Sxx, sem merge por agentes. Título e corpo descrevem a entrega final, evidências e limites; máximo alvo de 80 arquivos selecionáveis para caber no teto CodeRabbit de 100. Não remover código, testes ou docs necessários para esconder escopo do revisor. Corpus/evidência gerados têm proveniência e gerador revisáveis. Cada feature tem .md e linha na matriz; status globais reconciliados pelo coordenador.

Após push/criação, chamar link_pull_request imediatamente. O agente entrega URL, head SHA, testes executados e findings remanescentes e encerra, sem polling, watch, sleep ou merge. O coordenador assume a PR, lê os comentários, monitora por eventos e cria nova delegação para cada rodada de correção/recheck necessária. Não usar mensagem a thread finalizada como revisão nova.

Aceite do coordenador exige: head atual, checks relevantes verdes, revisão CodeRabbit realmente executada no head (ou validação explícita da revisão incremental), nenhum finding acionável aberto sem justificativa verificável, teste proporcional após correções/rebase e conferência independente da entrega. SKIPPED/NEUTRAL por quota/tamanho não é aprovação. Sem autoaprovação de GitHub com a conta autora. O usuário autorizou merge somente depois deste ciclo nesta rodada.

## Recursos e execução

- Cargo apenas pelo helper serial `/Users/wallyssonnunes/.cache/tessel-editor/cargo-serial.py`, executado no próprio worktree; ele chama ./scripts/cargo. Target próprio por worktree com cache mbx compartilhado; nunca target único para os dez. Não rodar gc/clean nem trocar HOME em wrappers.
- GPU/Chrome/prova nativa sob `/tmp/tessel-scene-gpu.lock`; adquirir GPU antes de Cargo quando ambos forem usados. Testes proporcionais por frente; gate completo do coordenador após integração.
- Dependências Node: usar pnpm e runtime existente; não instalar no repositório de origem. Não abrir/editar projetos ou janelas do usuário. Instância própria com HOME curto apenas ao executar binário direto.
- Artefatos próprios <=2 GiB por frente; parar geração/build pesado se livre <12 GiB e avisar. Não rodar ZIP64 4GiB, soak longo ou splat enorme sem coordenação. Não apagar temporários de outro agente.
- Branch/worktree preparados pelo T3. Verificar remote journey-studios/tessel-editor e cwd isolado antes de editar. Não criar checkout/worktree em shell, não escrever no canônico /Users/wallyssonnunes/www/tessel-editor, no tesselapp nem no checkout de outro agente.
