# redragon-fizz-k617-rgb

Controle open source do RGB do teclado **Redragon Fizz K617 (ABNT2)** no Linux, sem o software
oficial do Windows. O firmware do teclado não é alterado: o programa só envia os mesmos comandos
HID que o software oficial envia.

O protocolo foi obtido por engenharia reversa do `KeyboardDrv.exe` v1.6.6 (instalador oficial)
e testado no teclado.

## Instalação

Requer Rust (`cargo`) e Linux com hidraw. O programa não tem dependências externas.

```bash
git clone https://github.com/MarceloAntonio/redragon-fizz-k617-rgb.git
cd redragon-fizz-k617-rgb
cargo build --release
sudo cp target/release/k617rgb /usr/local/bin/
```

Para usar sem root, crie a regra udev e reconecte o teclado:

```bash
echo 'KERNEL=="hidraw*", ATTRS{idVendor}=="258a", ATTRS{idProduct}=="0049", TAG+="uaccess"' \
  | sudo tee /etc/udev/rules.d/70-k617.rules
sudo udevadm control --reload && sudo udevadm trigger
```

## Uso

```
$ k617rgb -h
uso: k617rgb [--dry-run] <comando>

  dump                                   mostra os 136 bytes de config
  mode <1-20> [--color RRGGBB|random] [--speed 1-4] [--brightness 0-4]
  off                                    brilho 0 no modo atual
  keys <grupo 1-5> <RRGGBB> [tecla=RRGGBB ...]   cor por tecla (modo 21)

modos:
   1 fixo
   2 respirar
   3 arco-íris
   4 piscante
   5 pingos de chuva
   6 roda arco-íris
   7 ondas
   8 estrelas
   9 sombra
  10 cobra
  11 fluxo neon
  12 reação
  13 onda senoidal
  14 varredura
  15 moinho
  16 cachoeira
  17 florido
  18 furacão
  19 colisão
  20 perfeito
```

Exemplos:

```bash
k617rgb mode 1 --color ff8800 --brightness 4        # fixo, laranja, brilho máximo
k617rgb mode 2 --color random --speed 3             # respirar, cores aleatórias
k617rgb off                                         # apaga (brilho 0)
k617rgb keys 1 000000 w=ff0000 a=ff0000 s=ff0000 d=ff0000   # só WASD vermelho
k617rgb dump                                        # mostra a config atual
k617rgb --dry-run mode 1 --color 00ff00             # só imprime os pacotes, sem enviar
```

Nomes de teclas aceitos no `keys`: `esc`, `1`-`0`, `-`, `=`, `backspace`, `tab`, `q`…`p`, `[`, `]`,
`\`, `capslock`, `a`…`l`, `;`, `'`, `enter`, `lshift`, `<`, `z`…`/`, `?`, `rshift`, `lctrl`,
`lwin`, `lalt`, `space`, `ralt`, `fn`, `app`, `rctrl`.

## Protocolo (HID feature reports, VID 258a / PID 0049)

| Pacote | Bytes | Significado |
|---|---|---|
| report 5, 6 B | `05 83 B6 00 00 00` | pede a config; resposta via GET report 6 (`[1]==0x83`, config em `[8..144]`) |
| report 6, 1032 B | `06 03 B6 00 00 00 00 00` + cfg | grava a config |
| report 6, 1032 B | `06 08 B8 00 40 00 00 00` + paleta | 21 B (7 cores RGB) por modo; o modo `m` fica em `8+21*m` |
| report 6, 1032 B | `06 09 BC/C0 00 40 00 00 00` + 1024 B | cor por tecla: 5 grupos × planos B,G,R × 126 LEDs |
| report 6, 1032 B | `06 04 D4 …` / `05 84 D4 …` | matriz de teclas (remapeamento), não usado aqui |

Config (`cfg`, 136 bytes): `[12]` = 1 se modo custom, `[13]` = modo (1-20, 21 = custom),
`[14]` = 0x20 + grupo custom, `[0x1c+2m]` = índice de cor (nibble baixo, 7 = aleatória),
`[0x1d+2m]` = velocidade<<4 | brilho, `[0x46..0x48]` = `5A A5`.

`docs/device.ini` é o arquivo de configuração extraído do instalador oficial: lista de modos,
ordem RGB e o mapa tecla → LED.

Pontos ainda não confirmados: a escala de velocidade (1-4) e como o modo "OFF" do software
oficial funciona (aqui, `off` só aplica brilho 0). Se o teclado se comportar de forma estranha,
segure **Fn+Esc** por alguns segundos para restaurar o padrão de fábrica.
