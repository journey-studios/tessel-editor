# Fundação do core e contrato de features

## Entrega

- Regras e testes de `crates/desktop/src/state/` extraídos para `crates/core/src/state/`.
- Parser semântico extraído para `crates/core/src/commands.rs`; desktop reexporta o mesmo parser.
- Biblioteca pública `tessel-editor-core`, consumida pelo desktop por dependência de caminho.
- Executável `tessel-headless` e wrapper `scripts/headless`, JSONL sem UI/socket.
- Matriz F001–F020, documento por feature, template, índice de docs e arquitetura.
- Skill local `.agents/skills/tessel-feature-contract/SKILL.md`, obrigatória pelo `AGENTS.md`.

## Validação nesta entrega

- `./scripts/cargo check --workspace --locked`: passou após atualizar o lock para o crate local.
- `cargo fmt --all -- --check`: passou.
- `./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings`: passou.
- `./scripts/cargo test --workspace --locked`: **101 testes**, sendo 38 desktop, 61 core
  e 2 integrações públicas/headless. Nenhum dos 99 testes anteriores foi removido.
- `./scripts/cargo build --workspace --locked`: passou.
- `./scripts/cargo tree -p tessel-editor-core --locked`: apenas `serde_json` e dependências
  transitivas de JSON; sem GPUI, windowing ou desktop.
- 13 exemplos JSONL dos contratos F001–F013 executados individualmente em processos headless:
  todos retornaram sucesso, inclusive o `status` após cada operação. Isso valida o exemplo,
  não todas as combinações possíveis da feature, por isso a matriz preserva I onde adequado.
- Wrapper `./scripts/headless` exercitado com seek em 120 frames.
- Links locais dos novos contratos conferidos.
- Skill validada com `quick_validate.py` do `skill-creator`, usando PyYAML em ambiente
  temporário do `uv` (sem alterar dependências Python do projeto).

Os dois testes novos comparam API direta e JSON, rejeição sem mutação, continuidade de
sessão entre linhas, saída não zero após erro, JSON inválido e rejeição de alvo desktop.
Os testes GPUI existentes continuam verificando os adapters de input e Dock.

## Limites

Não houve revisão visual nova nem drag manual. Não foi integrado motor de mídia. O modelo
mantém fixtures, IDs estáticos e campos públicos herdados, detalhados em `core-architecture.md`.
Não há criação dinâmica de tracks, split interno, efeitos processados, playback, persistência
ou exportação. A CLI headless ainda não tem consultas `tree/get/actions`; consultas detalhadas
estão disponíveis pelos tipos Rust. Sem commit, push ou mudanças no repositório `tesselapp`.
