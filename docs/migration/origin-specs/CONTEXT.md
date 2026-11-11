# Produção audiovisual de cenas ilustradas

Vocabulário do domínio de produção audiovisual do Tessel. Este glossário descreve os conceitos; os contratos e as decisões de implementação vivem na spec e no catálogo de padrões deste diretório.

## Composição

**SceneTemplate (template de cena)**:
Modelo reutilizável dos papéis espaciais de uma cena, como fundo, sujeitos, apoios e primeiro plano. Não contém a atuação ou a montagem de um filme específico.
_Evitar_: take, preset de câmera

**SceneDefinition (definição da cena)**:
Composição concreta de um espaço e seus elementos, com relações de posição, apoio e contato.
_Evitar_: shot, bloco de narração

**SceneInstance (instância da cena)**:
Ocorrência narrativa de uma cena, com estado e passagem de tempo próprios. Pode continuar existindo enquanto outra imagem ocupa a tela.
_Evitar_: recriar a cena a cada fala

**LayerRole (papel da camada)**:
Função visual de um elemento na composição, como fundo, sujeito, primeiro plano ou atmosfera. Papel visual não determina sozinho a profundidade física.

**ContactGroup (grupo de contato)**:
Conjunto de elementos cujo vínculo precisa permanecer visível e coerente, como mão e objeto ou personagem e assento. Mantém as peças isoladas; o grupo organiza as transformações e relações.
_Evitar_: achatar itens obrigatórios num PNG; animar sem vínculo elementos em contato

**Item lógico**:
Elemento identificável, como personagem, estaca ou cadeira. Sua identidade não depende de ser desenhado por uma ou várias peças.

**Fragmento visual**:
Peça desenhável de um item. Palma e dedos podem pertencer ao mesmo personagem e ocupar ordens diferentes em relação ao objeto segurado.

**AssetSet (pacote de assets)**:
Revisão coerente de itens, fragmentos, variantes, registro, contatos, proveniência e evidência. Mudanças invalidam os dependentes afetados, sem misturar versões silenciosamente.

**Referência canônica**:
Arte escolhida para identificar um personagem ou objeto numa revisão. Pode ser gerada antes da composição ou selecionada a partir de material já preparado, desde que cubra as vistas e regiões necessárias.

**Grafo de oclusão**:
Relações de frente/atrás entre fragmentos num estado de pose/contato. É separado da hierarquia que governa suas transformações.

**RevealRequirement (requisito de áreas reveladas)**:
Regiões que precisam existir para poses, estados de contato e trajetória planejados, com margem e evidência de cobertura. Inclui intermediários; só os extremos podem não bastar.

## Direção

**CameraPreset (receita de câmera)**:
Padrão reutilizável de movimento ou enquadramento, com intenção e limites próprios.
_Evitar_: cena, função editorial

**CameraClip (trajetória de câmera)**:
Movimento de câmera situado no tempo, que pode atravessar várias falas e janelas editoriais.
_Evitar_: progresso reiniciado por shot

**PerformancePreset (receita de atuação)**:
Padrão reutilizável de gesto, ciclo ou mudança de pose de um elemento da cena.
_Evitar_: movimento de câmera, rig obrigatório

**ShotRecipe (receita de plano)**:
Padrão de intenção editorial, como apresentar um local, revelar algo ou mostrar uma reação. Não impõe uma quantidade fixa de cortes por fala.
_Evitar_: template espacial, trajetória de câmera

**Shot (plano ou take)**:
Intervalo visual da montagem que apresenta uma cena por uma câmera. Ele não reinicia o tempo da cena ou da atuação.
_Evitar_: frase narrada, cena inteira

**EditJoin (emenda editorial)**:
Relação entre intervalos consecutivos da montagem, indicando continuidade, corte ou transição.

## Reprodução

**RenderProfile (perfil de render)**:
Conjunto de escolhas visuais e de entrega que caracteriza uma versão reproduzível da produção.

**RenderBundle (pacote de reprodução)**:
Revisão completa e congelada do material e das decisões necessários para reproduzir a produção.
_Evitar_: histórico de comandos do agente

**Passe por item**:
Imagem isolada ou diagnóstico de um item num frame/câmera. Não garante recomposição por source-over quando seus fragmentos se intercalam com outros itens. Contribuições calculadas para soma são um contrato distinto e dependem do restante da cena.

**Passe ordenado de camada**:
Fragmento ou conjunto contíguo na ordem global, com estágio, alpha, blend e espaço de cor declarados. A recomposição ocorre no estágio definido antes dos efeitos que misturam camadas.

**Imagem de referência**:
Imagem usada para orientar identidade, estilo ou composição. Sua função é distinta de um ponto de controle temporal de uma animação.
_Evitar_: usar keyframe sem especificar se significa imagem ou ponto de animação

**BundleEdit (edição de pacote)**:
Uma entrada da lista que `editBundle` aplica a um RenderBundle: ajuste de propriedade por caminho, ou comando de câmera, narração, grupos, rig ou caminho. A lista é aplicada em ordem, tudo ou nada, e o resultado passa pelas mesmas verificações de qualquer escrita. A CLI `scene`, o Studio e os scripts usam esta mesma entrada ([API da engine](scene-engine-api.md)).
_Evitar_: editar o `render-bundle.json` à mão

**Plano de áudio (`audioMixPlan`)**:
O que soa na edição: a mixagem recortada à janela com seus trechos silenciados e cada take que toca, com seu lugar na linha do tempo. O Studio toca e o render mixa a partir dele, então o MP4 soa como o Studio.
