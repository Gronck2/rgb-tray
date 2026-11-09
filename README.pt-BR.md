# RGB Tray

[English](README.md) · [Русский](README.ru.md) · [简体中文](README.zh-CN.md) · [Español](README.es.md) · [Deutsch](README.de.md) · [Français](README.fr.md) · **Português (Brasil)** · [日本語](README.ja.md)

Um pequeno aplicativo de bandeja do Windows que controla a iluminação RGB de teclados, mouses e luminárias diretamente via HID (e lâmpadas Tuya pela rede local), sem nenhum software do fabricante (G HUB, Synapse, Armoury Crate, SignalRGB) rodando em segundo plano.

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="Painel da bandeja">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="Editor por tecla">
</p>

## Dispositivos compatíveis

| Dispositivo | Conexão | O que funciona |
|---|---|---|
| Logitech G915 TKL | Receptor LIGHTSPEED | Estático, respiração, espectro, desligado; cores por tecla; bateria |
| Razer Naga V2 Pro | Dongle HyperSpeed / dock | Zonas separadas do logotipo e da roda de rolagem; efeitos; bateria |
| Razer Mouse Dock Pro | USB | Estático, respiração, espectro, desligado |
| Cerca de 90 outros mouses, teclados e acessórios Razer (Basilisk, DeathAdder, Viper, Cobra, BlackWidow, Huntsman, Ornata, Firefly, Goliathus, Base Station…), a partir dos dados do OpenRazer: [lista](profiles) | USB / dongle | Estático, respiração, espectro, desligado; zonas dos mouses; bateria. **Não testados**: o card do dispositivo avisa isso, e um relato é bem-vindo. Teclados sem fio HyperSpeed, por enquanto, só no cabo. |
| Teclado de notebook ASUS (Aura) | Integrado | Estático, respiração, espectro, desligado; zonas do teclado e luzes do corpo via LampArray |
| Geekboard MacroPad v2 (QMK/VIA) | USB | Estático, respiração, espectro, desligado |
| Outros teclados QMK com VIA e RGB (Keychron, teclados customizados…) | USB | Estático, respiração, espectro, desligado, brilho (respiração e espectro desenhados pelo aplicativo) |
| Dispositivos com o padrão Iluminação Dinâmica do Windows (HID LampArray) | USB / integrado | Estático, respiração, espectro, desligado, zonas informadas pelo dispositivo (respiração e espectro desenhados pelo aplicativo) |
| Lâmpadas Wi-Fi Tuya (Smart Life), por exemplo LINUS A60 | Wi-Fi, rede local | Cor, branco (uma cor quase branca usa os LEDs brancos e sua tonalidade), respiração, espectro, desligado; não apaga junto com a tela. Precisa da chave local da lâmpada, veja abaixo |

Outros modelos que usam os mesmos protocolos podem funcionar, mas não foram testados.

### Lâmpadas Tuya

O RGB Tray controla uma lâmpada Tuya pela rede local (protocolo 3.3) usando a *chave local* (local key) da lâmpada, que só a nuvem da Tuya fornece. Obtenha-a uma única vez, por exemplo com o [tinytuya](https://github.com/jasonacox/tinytuya) (`python -m tinytuya wizard`) ou com o login por QR code do Smart Life do `tuya-device-sharing-sdk`. Depois, liste as lâmpadas em `%APPDATA%\rgbtray\tuya.toml`:

```toml
[[device]]
name = "Luminária da mesa"
id = "bf1234567890abcdef12"
local_key = "0123456789abcdef"
```

O aplicativo encontra a lâmpada na rede local sozinho e memoriza o endereço dela (`local_ip`). A lâmpada precisa estar no **Smart Life** ou no **Tuya Smart**: contas de versões de marca desse aplicativo não conseguem fornecer as chaves. Uma lâmpada aceita apenas uma conexão local por vez: enquanto o app Smart Life estiver aberto em um celular, o RGB Tray não consegue acessá-la.

### Pedindo suporte a um novo dispositivo

Clique com o botão direito no ícone da bandeja → **Request device support…** (Solicitar suporte a dispositivo). O RGB Tray lista seus dispositivos HID, marca os que podem ter iluminação controlável, copia o relatório para a área de transferência e abre o [formulário de solicitação](../../issues/new?template=device-request.yml). Informe o modelo e cole o relatório. Os números de série no relatório aparecem encurtados.

Um dispositivo Razer ou QMK/VIA, ou um com protocolo simples, muitas vezes pode ser adicionado na hora, sem uma nova versão: basta descrevê-lo em um pequeno arquivo de perfil ([docs/profiles.md](docs/profiles.md)).

### Adicionando seu dispositivo com o Claude

Com o [Claude Code](https://claude.com/claude-code), você mesmo pode fazer um dispositivo funcionar, sem saber Rust nem HID. Clone este repositório, abra o Claude Code nele e digite `/add-device`. O Claude lê o relatório do dispositivo, procura o protocolo (OpenRGB, OpenRazer, uma captura USB do aplicativo do fabricante), escreve um perfil ou um driver e testa junto com você: como ele não enxerga as luzes, pergunta o que o dispositivo fez depois de cada etapa. Comandos que poderiam danificar um dispositivo (atualizações de firmware, gravação na memória flash) são proibidos. Quando tudo funcionar, o Claude pode enviar o resultado como pull request, para que o suporte venha embutido para todo mundo.

## Recursos

A interface do aplicativo está disponível apenas em inglês e russo.

- Um único painel para todos os dispositivos: cor, efeito, brilho, para todos de uma vez ou para cada dispositivo.
- Brilho global e, por cima dele, um brilho por dispositivo.
- Velocidade da respiração e do espectro, global e por dispositivo.
- Efeitos de firmware de um teclado no editor de teclas dele, ao lado das predefinições: onda (wave) e ondulação (ripple) no Logitech G915. Pintar uma tecla traz de volta as cores das teclas.
- Editor por tecla para o teclado Logitech: pincel, conta-gotas, arco-íris, degradê de duas cores, predefinições WASD e só letras, destaque de uma palavra, esquemas salvos, layouts ANSI/ISO.
- Predefinições com atalhos de teclado globais (incluindo F13–F24 com Ctrl/Alt/Shift/Win).
- Um atalho global por dispositivo que o desliga e o liga de novo como estava (o botão de teclado depois do nome no card do dispositivo: F13–F24 ou qualquer combinação), por exemplo para uma luminária do quarto. As predefinições não são afetadas.
- Vários dispositivos idênticos com configurações separadas; os dispositivos são reconhecidos pelo número de série.
- Dispositivos desconectados ficam guardados em um arquivo com suas configurações e voltam quando são reconectados.
- Status "No link" (sem conexão) quando um dispositivo sem fio está em repouso ou desligado; as cores são reaplicadas quando ele desperta.
- Reaplica a iluminação depois da suspensão (incluindo o Modern Standby) e quando dispositivos são conectados.
- Atualizações: uma vez por dia o aplicativo consulta o GitHub em busca da versão mais recente; se houver uma versão nova, aparece um card no painel que a instala com um clique. A verificação pode ser desativada no menu da bandeja.
- Acompanha a tela: a iluminação escurece quando o Windows escurece a tela e se apaga enquanto a tela está desligada (inatividade, bloqueio, suspensão). Pode ser desativado no menu da bandeja.
- Alertas de bateria fraca em 10% e 5%.
- O ícone da bandeja mostra a cor atual.
- Modo fixado: o painel fica aberto por cima das outras janelas e pode ser arrastado pelo cabeçalho.
- Exportação e importação de todas as configurações para um arquivo. As predefinições podem ser compartilhadas uma a uma: clique com o botão direito em uma predefinição → Save to a file… (Salvar em um arquivo…), e quem recebe a carrega com Load settings or a preset… (Carregar configurações ou uma predefinição…) no menu da bandeja; ela é aplicada aos dispositivos dessa pessoa dos mesmos modelos (a todos, se houver vários), e o aplicativo informa quais modelos do arquivo ela não tem.
- Interface em inglês e russo: em russo quando o Windows ou um de seus layouts de teclado está em russo, em inglês nos demais casos; dá para trocar no menu da bandeja.
- Tema claro e escuro, acompanhando o Windows.

## Instalação

1. Baixe `RGBTray-Setup-<version>.exe` em [Releases](../../releases).
2. Execute o arquivo. O aplicativo se instala em `%LOCALAPPDATA%\Programs\RGB Tray` para o usuário atual, sem direitos de administrador, e é iniciado.
3. Ele pode ser removido em Configurações do Windows → Aplicativos.

O instalador ainda não tem assinatura de código, então o Windows SmartScreen pode exibir um aviso (Mais informações → Executar assim mesmo). As configurações ficam em `%APPDATA%\rgbtray` e são mantidas em atualizações e reinstalações. A pasta `backups` dentro dela guarda cópias das configurações dos últimos 14 dias e de antes de cada atualização; se o arquivo de configurações for danificado, o aplicativo usa sozinho a última cópia que funciona.

Feche o software de iluminação do fabricante antes de usar o RGB Tray: dois programas escrevendo no mesmo dispositivo brigam entre si. No Windows 11, o serviço "Logitech LampArray" (Iluminação Dinâmica) faz o G915 ignorar efeitos; o aplicativo avisa sobre isso e oferece desativá-lo.

## Linha de comando

| Comando | O que faz |
|---|---|
| `rgbtray --diagnose [file.txt]` | Grava o relatório de dispositivos: versões do aplicativo e do Windows, dispositivos compatíveis e todos os dispositivos HID agrupados com suas interfaces, com os prováveis controladores de iluminação primeiro. Sem nome de arquivo, o relatório é salvo em `%APPDATA%\rgbtray\rgbtray-diagnose.txt` e aberto no Bloco de Notas. Também disponível no menu da bandeja. |
| `rgbtray --hid-send <out.txt> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"…` | Envia relatórios HID brutos e acrescenta as respostas ao arquivo, para descobrir o protocolo de um dispositivo ([docs/profiles.md](docs/profiles.md#hid-any-device-with-a-simple-protocol)). |
| `rgbtray --profile-check <out.txt> <profile.toml>` | Lê um arquivo de perfil sem acessar nenhum dispositivo e grava como o aplicativo interpreta cada modelo, ou o que há de errado com ele. |
| `rgbtray --profile-test <out.txt> <profile.toml>` | Reproduz cores e efeitos nos dispositivos de um perfil `[hid]` que ainda não foi instalado e registra cada etapa. |
| `rgbtray --show` | Inicia com o painel aberto e fixado. |
| `rgbtray --uninstall` | Remove o aplicativo (as configurações são mantidas). |

## Compilando a partir do código-fonte

Requer Windows 10/11 e [Rust](https://rustup.rs) estável, com a toolchain `x86_64-pc-windows-gnu` ou `-msvc`. Depois de instalar o rustup, abra um novo terminal para que o `cargo` esteja no PATH.

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

O instalador é o mesmo executável: com o nome `RGBTray-Setup-*.exe` (ou iniciado com `--install`), ele se instala sozinho.

## Como contribuir

Relatórios de dispositivos, testes no seu hardware, relatos de bugs e pull requests são bem-vindos. O [CONTRIBUTING.md](CONTRIBUTING.md) explica como compilar e verificar o aplicativo, como adicionar um dispositivo e o que um pull request precisa ter.

## Agradecimentos

Os protocolos dos dispositivos foram aprendidos com estes projetos; o RGB Tray não contém código deles:

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB): iluminação Logitech, Razer e ASUS Aura
- [OpenRazer](https://github.com/openrazer/openrazer): protocolo Razer
- [Solaar](https://github.com/pwr-Solaar/Solaar): Logitech HID++
- [QMK](https://qmk.fm) e [VIA](https://www.caniusevia.com): protocolo de iluminação do MacroPad

## Licença

Licenciado sob [MIT](LICENSE-MIT) ou [Apache-2.0](LICENSE-APACHE), à sua escolha.

Salvo declaração explícita em contrário de sua parte, qualquer contribuição enviada intencionalmente por você para inclusão neste projeto, conforme definido na licença Apache-2.0, será licenciada duplamente como acima, sem quaisquer termos ou condições adicionais.

Sem afiliação com Logitech, Razer, ASUS, Geekboard ou Tuya. As marcas registradas pertencem a seus respectivos proprietários.

Este documento é uma tradução; em caso de divergência, prevalece o README em inglês.
