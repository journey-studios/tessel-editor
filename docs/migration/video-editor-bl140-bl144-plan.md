# Recursos do editor para o vídeo — BL140–BL144

Autorização: usuário solicitou iniciar câmera com keyframes, títulos com acentos, fades de áudio, importação direta SHARP e miniaturas/cabeçalho, com Cursor Sonnet 5.5. Base verificada: main `11832a9` (PR19 integrada). Projeto: Rust/GPUI em `/Users/wallyssonnunes/www/tessel-editor`, distinto do app RPG em `.t3/projects`.

Status e atribuição vivem em [backlog.md](../backlog.md). Este documento define sequência, contratos e aceite; não é fila de execução paralela.

## Sequência e critérios

| Ordem | ID | Entrega | Aceite mínimo |
|---|---|---|---|
| 1 | BL140 | Keyframes de posição/alvo/FOV da câmera de clipe `tessel-scene@1`, interpolação/easing, dolly e push-in | Avaliação temporal no core; tempo local correto após trim/move; persistência/undo; API Rust, JSONL sem janela e controles semânticos da UI; preview e export com quadros reais e compatibilidade de cenas antigas. Reutilizar capacidades de câmera da cena dirigida onde fizer sentido, sem portar o motor inteiro. |
| 2 | BL141 | Títulos com acentos em português | Frases com á/é/ã/ç e formas Unicode compostas/decompostas; fonte determinística com licença e distribuição documentadas; layout/quebra/alinhamento preservados; resultado real em monitor, headless e export; sem substituir título por PNG manual. |
| 3 | BL142 | Fade-in/out de clipes de áudio | Contrato de duração, extremos, sobreposição e trim no core; edição lib/JSONL/UI; envelopes verificáveis por amostras/RMS no áudio renderizado/exportado; ausência de fade preserva saída; sem processar áudio na thread UI. |
| 4 | BL143 | Importação direta do PLY SHARP | Usuário importa arquivo sem conversão manual externa; eixos, orientação/up, FOV/intrínsecos e câmera compatíveis com a origem; metadados ausentes ou ambíguos tratados explicitamente; fixtures sintéticas, erro com estado intacto, projeto reaberto/pacote/export. Depende do contrato de câmera BL140; não autoriza gerar imagens, baixar/rodar modelo SHARP, nem serviço remoto. |
| 5 | BL144 | Cabeçalho do projeto e miniaturas reais de splat/cena | Abrir projeto pela CLI ou GUI atualiza o mesmo cabeçalho; troca de projeto não aplica thumbnail atrasada do anterior; miniaturas renderizadas de dados reais por jobs/adapters, com cancelamento/cache e sem bloquear UI; estado semanticamente inspecionável e captura da janela nativa. |

As entregas 2 e 3 são independentes da primeira; a execução sequencial é uma escolha de recursos, não dependência de domínio. BL144 não deve alterar o medidor em revisão na PR18. Não expandir esta rodada para outras pendências de desempenho, cor ou S10.

## Ownership e integração

Um implementador, sem subagentes, trabalha somente no checkout isolado preparado pelo T3. Ele é responsável pelas alterações necessárias dos cinco IDs, em sequência, e entrega uma PR por ID. Para tarefas independentes, branch da main atual; BL143 pode usar uma PR empilhada sobre BL140, com base e links explícitos. Não criar worktrees adicionais pelo shell. Não editar a origem, o projeto aberto ou o backlog não commitado da raiz.

O coordenador é dono do backlog comum da raiz e integra os estados fornecidos pelo implementador. Atribuição não significa início: somente o item efetivamente começado fica Em andamento. Não declarar feature concluída sem prova por superfície. Novas pendências ganham subIDs do item correspondente e entram no backlog.

## Regras de execução e aceite

- Aplicar AGENTS.md e a skill local tessel-feature-contract. Core sem GPUI/wry; mídia pesada em crates/media; runtime desenha planos, sem regras de edição duplicadas.
- Controles novos: Command, árvore de estado, parser, handler compartilhado e testes. UI só scripts/ui; JSONL sem janela via scripts/headless. Socket da janela não comprova headless.
- Atualizar contrato próprio e matriz de features em cada entrega. Preserve IDs existentes. Commits via skill commit, sem trailers/referências a assistentes.
- Fmt, check/clippy/test/build via scripts/cargo conforme AGENTS; testes negativos relevantes e provas reais proporcionais. Serializar builds pesados e GPU, respeitar piso de 12 GiB e não executar clean/gc. Não interferir nas PR14/18.
- Preservar o histórico Git ancorado do S10: nenhuma alteração de livro/oráculo/tolerâncias/fixtures fixadas para obter PASS; exceções viram pendências explícitas.
- Não gerar mídia nova nem incluir dados/imagens privados. Provas usam corpus sintético ou cópias locais autorizadas. Licenças de fontes/dependências públicas devem acompanhar a implementação.
- Cada PR deve informar SHA, comandos realmente executados, resultados, limites e evidências lib/headless/UI. Criar/push implica link_pull_request como chamada seguinte; não monitorar nem mergear pelo filho.
- Após entrega, o coordenador revisa diffs/evidências e pede revisão independente. CodeRabbit por cota/NEUTRAL não vale aprovação; corrigir achados válidos e conferir novamente. Merge é decisão do coordenador conforme autorização vigente; esta delegação não concede merge ao filho.
