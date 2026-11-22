# Fonts

Nunito by The Nunito Project Authors (https://github.com/googlefonts/nunito), licensed under
the SIL Open Font License 1.1 (no Reserved Font Name): see `OFL-Nunito.txt`. Ruthenium's
out-of-game menus use it.

Nunito is published as a variable font (`ofl/nunito/Nunito[wght].ttf` in google/fonts,
SHA-256 `bb55a5ca5c2042335b3991af27c4d0705d0ef41cac6164ac737fd8f2a1e85207`). Variable fonts
keep overlapping contours, which distance fields render with specks where strokes overlap,
so the bundled files are static instances made from it with fontTools 4.60.1 and
skia-pathops 0.9.2, overlaps removed:

```sh
fonttools varLib.instancer 'Nunito[wght].ttf' wght=600 --static --remove-overlaps \
    --update-name-table -o Nunito-SemiBold.ttf
```

| File | wght | SHA-256 |
|---|---|---|
| `Nunito-Regular.ttf` | 400 | `fd66dc00667c4f1428a3f3c1ba4fa06d9cfd5f5ddce93219313100b1b50309d7` |
| `Nunito-Medium.ttf` | 500 | `cf853d8e358abfeca11e42a10879276e5a8d09d5f966f2979ef4895ad0d3fec0` |
| `Nunito-SemiBold.ttf` | 600 | `59e9ef4a6004bc379aa7e74bb73a342da4f7b74820ce732f44e7a230f465ce4f` |
| `Nunito-ExtraBold.ttf` | 800 | `65834b451cb09fe07acfd837b2129c22638e9afd16c070299c4c3f0c0eecedf4` |
