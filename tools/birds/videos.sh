#!/bin/sh
# Bird videos into target/refs/ab/: real-vs-ours species by species (birds_species_ab1..5.mp4)
# and the demo (birds_species_demo1..3.mp4: each species alone, then the choruses).
#   sh tools/birds/videos.sh <ab_dir> <demo_dir>
# <ab_dir> and <demo_dir> hold `render_birds ab` and `render_birds demo` output.
HERE=$(cd "$(dirname "$0")" && pwd)
R=/Users/joakimeriksson/work/gamesynth/target/refs/decoded/birds
V=/Users/joakimeriksson/work/gamesynth/target/refs/scripts/ab_video.sh
OUT=/Users/joakimeriksson/work/gamesynth/target/refs/ab
AB=$1
D=$2
start() { sh "$HERE/py.sh" first_song.py "$1"; }
o() { echo "$AB/$1.wav $(start "$AB/$1.wav")"; }
set -e
# shellcheck disable=SC2046
sh "$V" "$OUT/birds_species_ab1.mp4" \
  "BLACKBIRD - REAL" "$R/blackbird/811988_richwise.wav" 4.4 -- "BLACKBIRD - BRUSVERK" $(o blackbird) -- \
  "ROBIN - REAL" "$R/robin/671058_Andrew.soundscape.wav" 2.3 -- "ROBIN - BRUSVERK" $(o robin) -- \
  "CHAFFINCH - REAL" "$R/chaffinch/332851_Travesia_Sonora.wav" 0 -- "CHAFFINCH - BRUSVERK" $(o chaffinch)
sh "$V" "$OUT/birds_species_ab2.mp4" \
  "GREAT TIT - REAL" "$R/great_tit/418102_straget.wav" 0.8 -- "GREAT TIT - BRUSVERK" $(o great_tit) -- \
  "CUCKOO - REAL" "$R/cuckoo/688844_Elanor1995.wav" 29.8 -- "CUCKOO - BRUSVERK" $(o cuckoo) -- \
  "WOOD PIGEON - REAL" "$R/wood_pigeon/814890_Sadiquecat.wav" 25.8 -- "WOOD PIGEON - BRUSVERK" $(o wood_pigeon)
sh "$V" "$OUT/birds_species_ab3.mp4" \
  "COLLARED DOVE - REAL" "$R/collared_dove/610563_randomthoughts7.wav" 0 -- "COLLARED DOVE - BRUSVERK" $(o collared_dove) -- \
  "CROW - REAL" "$R/crow/181088_cfrooos.wav" 10.8 -- "CROW - BRUSVERK" $(o crow) -- \
  "TAWNY OWL - REAL" "$R/tawny_owl/745208_Patrick_Corra.wav" 0 -- "TAWNY OWL - BRUSVERK" $(o tawny_owl)
sh "$V" "$OUT/birds_species_ab4.mp4" \
  "HOUSE SPARROW - REAL" "$R/house_sparrow/404663_straget.wav" 14.4 -- "HOUSE SPARROW - BRUSVERK" $(o house_sparrow) -- \
  "HERRING GULL - REAL" "$R/herring_gull/538016_Canardo55.wav" 8.3 -- "HERRING GULL - BRUSVERK" $(o herring_gull) -- \
  "NIGHTINGALE - REAL" "$R/nightingale/521035_smand.wav" 0.6 -- "NIGHTINGALE - BRUSVERK" $(o nightingale)
sh "$V" "$OUT/birds_species_ab5.mp4" \
  "SKYLARK - REAL" "$R/skylark/387426_Kinoton.wav" 7.6 -- "SKYLARK - BRUSVERK" $(o skylark) -- \
  "WOODPECKER DRUM - REAL" "$R/woodpecker/428146_naturenotesuk.wav" 2.2 -- "WOODPECKER DRUM - BRUSVERK" $(o woodpecker)
sh "$V" "$OUT/birds_species_demo1.mp4" \
  "BLACKBIRD" "$D/blackbird.wav" -- "ROBIN" "$D/robin.wav" -- "CHAFFINCH" "$D/chaffinch.wav" -- \
  "GREAT TIT" "$D/great_tit.wav" -- "CUCKOO" "$D/cuckoo.wav" -- "WOOD PIGEON" "$D/wood_pigeon.wav"
sh "$V" "$OUT/birds_species_demo2.mp4" \
  "COLLARED DOVE" "$D/collared_dove.wav" -- "CROW" "$D/crow.wav" -- "HOUSE SPARROW" "$D/house_sparrow.wav" -- \
  "SKYLARK" "$D/skylark.wav" -- "HERRING GULL" "$D/herring_gull.wav" -- "WOODPECKER" "$D/woodpecker.wav"
sh "$V" "$OUT/birds_species_demo3.mp4" \
  "NIGHTINGALE" "$D/nightingale.wav" -- "TAWNY OWL" "$D/tawny_owl.wav" -- \
  "GARDEN MORNING CHORUS" "$D/chorus_garden.wav" 0 -- "GARDEN MORNING CHORUS (CONT.)" "$D/chorus_garden.wav" 8 -- \
  "DAWN CHORUS" "$D/chorus_dawn.wav" 2 -- "NIGHT WOODS" "$D/chorus_night.wav" 2
