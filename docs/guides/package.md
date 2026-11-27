# Arquivar um projeto editável com seus assets

Salve o projeto e rode o comando com um destino novo.

```sh
./scripts/tessel package aventura.tessel.json aventura.tesselpack
```

O pacote leva o documento editável e as mídias usadas, incluindo splats e áudio. O editor verifica o pacote antes de colocá-lo no destino. Mídia ausente ou alterada impede a criação de um pacote completo.

- Use `--format directory` para criar uma pasta de pacote.
- Use `--sequence ID` para levar só uma sequência e suas dependências. Repita a opção para incluir várias.
- Use `--replace` para trocar um pacote existente após a verificação.

O progresso sai em stderr e o resultado JSON em stdout. [Restaurar o pacote](restore.md) · [Contrato de empacotamento](../features/package.md)
