# Plano de implementação — componentes do editor

> **Histórico.** Escrito antes da migração para GPUI Kit (ver `gpui-kit-migration-report.md`). Menções a Guise, Tailor, GPUI 0.2.2 e a controles manuais descrevem o estado anterior e não valem mais.

Status: pronto para implementação delegada. Stack: Rust + GPUI 0.2.2 + Guise + Tailor.

## Objetivo e limites

Aproximar a interface da referência Splice fornecida pelo usuário, priorizando
microcomponentes densos e reutilizáveis. **Somente um monitor central.** Biblioteca
à esquerda, monitor e timeline no centro, inspetor à direita. Cabeçalho unificado
com controles nativos de janela. Não criar um segundo monitor.

O usuário autorizou interação real nos componentes: pastas, accordions, sliders,
selects, tabs, seleção e drag and drop de tracks/clipes. Essas interações usam
estado demonstrativo em memória. Não migrar Three.js, reprodução/exportação real,
processamento de áudio/vídeo, persistência ou integrações. Não confundir uma forma
de onda ilustrativa com áudio processado. Todos os controles implementados devem
ser acionáveis pela CLI na mesma instância e pelo mesmo handler usado pela UI.

## Referências locais

- Composição completa: `/Users/wallyssonnunes/.t3/userdata/attachments/af7eb47d-f7ee-4fd2-adde-cac8a54313ea-30ebc81f-1abc-4bea-a7f0-c43dfeb8394a.png`
- Pastas: `/Users/wallyssonnunes/.t3/userdata/attachments/af7eb47d-f7ee-4fd2-adde-cac8a54313ea-0eaf8865-7ed9-4275-a3c2-fcb5e1cd6e30.png`
- Inspetor: `/Users/wallyssonnunes/.t3/userdata/attachments/af7eb47d-f7ee-4fd2-adde-cac8a54313ea-b46e95f1-8cd1-4609-88ca-e1e1d87619b7.png`
- Timeline: `/Users/wallyssonnunes/.t3/userdata/attachments/af7eb47d-f7ee-4fd2-adde-cac8a54313ea-b126d329-664b-4308-b4f5-7e94cf49b99e.png`

As imagens `assets/demo/{highlands,coast}.png` foram baixadas dos assets da demo
pública ProUI somente como referências de protótipo. Registrar origem e escopo;
não presumir licença de redistribuição comercial. O kit ProUI não é dependência.

## Componentes e critérios de aceite

| ID | Componente | Implementar e verificar | Operação sem mouse (forma sugerida) |
|---|---|---|---|
| C01 | Tokens de workstation | Superfícies cinza, bordas de 1 px, espaçamento compacto, raio pequeno, hierarquia de texto; acentos discretos violeta/verde/teal; usar tokens Guise. Evitar grandes cards e pills para representar tracks. | `ui theme dark/light`; estado do tema real. |
| C02 | Shell de painéis | Cabeçalho/menubar/toolbar compactos; biblioteca esquerda, monitor único e timeline central, inspetor direita, status inferior; sem overflow na janela inicial e em uma janela menor. | IDs de regiões e estado de visibilidade/dimensões; ações nativas existentes preservadas. |
| C03 | IconButton e ToolbarGroup | Lucide uniforme, grupos com separadores, tooltip, foco, selecionado/desabilitado. Não habilitar ações de mídia inexistentes. | IDs semânticos e invoke para operações implementadas; erro explícito nas indisponíveis. |
| C04 | Árvore de pastas | Disclosure, ícone, rótulo, contagem à direita, indentação e guia; selecionar e expandir/recolher grupos; filhos acompanham visibilidade real. | `set folder:<id> expanded true/false`; `invoke ... select`; estado e contagem. |
| C05 | Biblioteca de mídia | Campo de busca, grade de miniaturas, nome/metadados, seleção destacada; busca filtra dados de demonstração. Seleção alimenta monitor/inspetor. | `set media.search value ...`; selecionar item por ID; consultar resultados. |
| C06 | Tabs | Abas do inspetor (Vídeo/Áudio/Cor) e modo inferior (Editar/Cor/Entregar); seleção real com conteúdo/estado coerentes. Modos não implementados mostram estado honesto. | `set inspector.tabs value video/audio/color`; `set workspace.tabs value ...`. |
| C07 | AccordionSection | Cabeçalho compacto com chevron, título e reset opcional; painel expande/recolhe sem apagar valores; suporte a teclado. | `set inspector.transform expanded ...`; reset semântico. |
| C08 | PropertyRow | Alinhamento consistente de label, controle, número e unidade; número legível e valor inválido rejeitado. Reutilizar nas propriedades. | `get property:<id>` com tipo/valor/unidade/limites. |
| C09 | Slider numérico | Trilho fino, thumb, valor e unidade; zoom, X/Y, rotação, end zoom, opacidade. Drag e teclado alteram o mesmo estado; clamp/validação definidos. | `set property:zoom value 110`; `set property:opacity value 75`; ler confirmação. |
| C10 | Select | Campo compacto com chevron e lista real: blend mode normal/multiply/screen etc.; teclado, selecionado e fechamento. | `set property:blend value multiply`; get lista de opções e valor atual. |
| C11 | Inspetor e thumbnail | Seções Transform/Composite/Cropping/Timing, miniatura e guias; reset por seção. A seleção da timeline troca os dados mostrados. Alterar valores de UI não promete renderização/exportação de vídeo. | Inspecionar/resetar propriedades por seção e clip selecionado. |
| C12 | Monitor único | Imagem de demonstração, metadados, timecode, seleção e controles de transporte explicitamente limitados; manter aspecto e centralização. | Consultar mídia selecionada/timecode; nenhum segundo monitor. |
| C13 | TimelineToolbar | Seleção/snap, nome da sequência, zoom da timeline; botões inativos para ferramentas ainda ausentes. Zoom e snap alteram estado real. | `set timeline.zoom value ...`; `set timeline.snap value true/false`. |
| C14 | TimeRuler e playhead | Régua com ticks/timecodes, marcador vertical contínuo alinhado às tracks; seek por clique/drag e limites da sequência; sem alegar playback real. | `set timeline.playhead frame 72`; get frame e fps. |
| C15 | TrackHeader e TrackLane | IDs/labels V2,V1,A1,A2; altura adequada ao tipo, seleção, visibilidade/mute; linhas e fundos contínuos, sem cartões soltos. | Selecionar/mutar/ocultar por ID; ler ordem e estado. |
| C16 | ClipBlock | Blocos retangulares proporcionais à duração, label, miniaturas repetidas, timecode; borda de seleção âmbar. Estados selecionado/arrastando/drop target. | Selecionar clip por ID; consultar track, início e duração. |
| C17 | Drag and drop | Reordenar tracks e mover clipes horizontalmente/entre tracks compatíveis; validar limites, snap e política de sobreposição explícita. Não permitir áudio em trilha de vídeo. Feedback de drop e cancelamento sem mudança residual. | `move clip:<id> --track <id> --frame N`; `move track:<id> --before <id>`; mesmos handlers do drag. |
| C18 | WaveformLane | Barras determinísticas ilustrativas dentro de clipes de áudio; tons verde/teal, label e seleção; sem dependência de processamento de áudio. | Estado de track/clip; identificar waveform como dado demonstrativo. |
| C19 | StatusBar | Duração, seleção, fps/resolução e indicação de demonstração; valores derivados do mesmo estado da UI. | `ui status` inclui estado real do editor. |
| C20 | Automação semântica | Evoluir tree/get/actions e adicionar set/move; árvore de componentes vivos, valores, opções, limites, seleção, expanded/visible/enabled; IDs semânticos estáveis, sem estado CLI paralelo. | JSON versionado, erros estruturados e exit !=0; janela aberta reflete ações. |

A sintaxe da CLI pode ser ajustada para manter um contrato pequeno e consistente;
os exemplos acima descrevem capacidades obrigatórias, não obrigam um parser específico.

## Ordem de entrega

1. **Fundação:** C01–C03 e C20; separar estado demonstrativo e handlers compartilhados.
   Preservar socket local e limites de acesso existentes. Atualizar testes da árvore:
   não depender de contagem fixa de nós estáticos para declarar cobertura da UI viva.
2. **Biblioteca e inspetor:** C04–C12. Completar interações, estados e comandos CLI de
   cada família antes de avançar. Reutilizar Guise; customizar wrappers compactos.
3. **Timeline:** C13–C18. Construir régua/lanes/clipes antes de drag and drop;
   ambos os caminhos (gesto e CLI) chamam comandos do mesmo modelo.
4. **Integração:** C19, ajuste de densidade/layout e verificação das referências.
   Marcar cada Cxx como concluído ou pendente com evidência em um relatório.

## Arquitetura e autoria

Manter Tailor como documento de composição editável e Guise como biblioteca.
Microcomponentes com comportamento podem ser módulos Rust próprios, compostos
pelo shell. Não editar manualmente o arquivo gerado: se a composição dinâmica
exigir adaptar o export, documentar a fronteira e preservar regeneração repetível.
Evitar que a árvore da CLI descreva nós estáticos que já não correspondam à UI.

Manter o projeto em `~/www/tessel-editor`; não alterar o Tessel de origem.
Trabalho local, sem publicação, deploy, push ou PR nesta etapa. O repositório foi
inicializado mas ainda não tem commit; todos os arquivos atuais são trabalho
pré-existente a preservar. Não descartar nem substituir o workspace inteiro.

## Validação

- `cargo fmt --all -- --check`, `cargo check --workspace --locked`,
  `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- Testes comportamentais para expandir pastas/accordions, seleção/tabs, validação
  numérica/select, clamp/seek/snap, reorder e movimento entre tracks compatíveis.
- Testar drag por comandos do modelo: movimento válido, drop incompatível, início
  negativo, fora de sequência, cancelamento e política de overlap escolhida.
- Validar CLI contra app aberto: set e move mudam o estado consumido pela UI;
  confirmar seleção, valores e ordem; unknown/disabled/invalid retornam erro.
- Conferir visualmente uma janela ampla e uma menor, sem interação por coordenadas.
  Observação/screenshot pode complementar a CLI, não substitui seus comandos.
- Não declarar funcionalidade de engine nem fidelidade visual sem evidência.
