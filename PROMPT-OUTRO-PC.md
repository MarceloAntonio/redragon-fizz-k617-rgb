Cole o texto abaixo no Claude Code do PC onde o teclado está conectado.
Antes, copie para lá a pasta `k617rgb` (ou faça push/clone) e o instalador `SOFTWARE K617RGB ABNT2.exe`.

---

Tenho um teclado Redragon K617 RGB ABNT2 (USB 258a:0049, chip Sinowealth) conectado por cabo.
Quero controlar o RGB dele no Linux com o programa Rust em `~/projetos/k617rgb`, que foi escrito
a partir de engenharia reversa estática do software oficial, mas ainda não foi testado no hardware.
Leia o `README.md` e o `src/main.rs` antes de tudo: o protocolo descoberto está documentado lá.

Faça nesta ordem, me mostrando o resultado de cada etapa:

1. **Instalar o REA globalmente** (https://github.com/morluto/rea). Verifique se tenho Node >= 22.19 e npm.
   Se o `npx` for bloqueado, me peça para rodar `! npx rea-agents setup` e escolher o Claude Code.
   Depois me lembre de reiniciar o Claude Code para o MCP carregar, e confirme que as ferramentas do REA aparecem.
2. **Instalar as dependências** (sou Arch; use pacman e me peça o sudo quando precisar):
   `innoextract ghidra wireshark-qt rust`. Adicione meu usuário ao grupo `wireshark` e carregue o módulo `usbmon`.
3. **Confirmar o dispositivo**: `lsusb -d 258a:0049` e liste os `/dev/hidraw*` dele. Mostre o
   `report_descriptor` de cada interface e confirme que existem os feature reports 5 (7 bytes) e 6 (1032 bytes).
   Configure a regra udev do README.
4. **Validar a leitura** (somente leitura, sem risco): `cargo build --release` e `k617rgb dump`.
   Confirme que a resposta começa com `06 83` e que `cfg[0x46..0x48] == 5A A5`. Se não bater, pare e use o REA.
5. **Conferir a engenharia reversa com o REA**: extraia o instalador com `innoextract` e analise o `KeyboardDrv.exe`
   (e o `HidParse.dll`) com o REA/Ghidra. Confira estes pontos que ficaram incertos:
   - funções em 0x446320 (grava config), 0x446420 (lê config), 0x446840 (paleta), 0x446550 (cor por tecla)
     e 0x40f600 (monta os bytes por modo). Endereços com base 0x400000; o Inno Setup 5.3.3 aplica um filtro
     de CALL/JMP no .exe, mas o innoextract já desfaz isso.
   - a escala de velocidade e brilho (SpeedHW/LightHW no INI), o nibble alto do byte de cor (direção?)
     e como funciona o modo "OFF" (RGBOFFIndex=21).
6. **Testar escrita, um passo por vez**, perguntando antes de cada um e me pedindo para olhar o teclado:
   `k617rgb mode 1 --color ff0000 --brightness 4`, depois `mode 2 --speed 1` e `--speed 4`, `off`, e
   `keys 1 000000 w=ff0000 a=ff0000 s=ff0000 d=ff0000`. Faça `dump` antes e depois de cada um e compare.
   Se algo falhar, use o Wireshark com o usbmon enquanto rodo o software original no wine
   (ou numa VM Windows com USB passthrough) para capturar os bytes reais e corrigir o código.
7. **Corrigir** o `src/main.rs` e o README com o que foi confirmado, manter o `cargo test` passando
   e me dar um resumo do que mudou. Se o teclado se comportar de forma estranha, me lembre do reset com Fn+Esc.
