#!/bin/sh
# Real-vs-ours spectrograms for every species, into target/refs/birds/compare/.
#   sh tools/birds/compare_all.sh <ours_dir> [species...]
# <ours_dir> holds the `render_birds species` output (<name>.wav, and <name>_2.wav for a second
# individual if present).
R=/Users/joakimeriksson/work/gamesynth/target/refs/decoded/birds
OUT=/Users/joakimeriksson/work/gamesynth/target/refs/birds/compare
O=$1; shift
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$OUT"
want() { [ -z "$ONLY" ] || echo " $ONLY " | grep -q " $1 "; }
ONLY="$*"
C="sh $HERE/py.sh compare.py"
want blackbird && $C "$OUT/blackbird.png" 8000 4 "Blackbird: real (2 birds) vs ours (2 renditions)" \
  "real=$R/blackbird/811988_richwise.wav@4.5" "real=$R/blackbird/725332_Sacha.Julien.wav@0.9" "ours=$O/blackbird.wav@auto" "ours=$O/blackbird.wav@auto12"
want blackbird && $C "$OUT/blackbird_alarm.png" 10000 3 "Blackbird alarm rattle: real vs ours" \
  "real=$R/blackbird/835517_Kanny100.wav@0" "ours=$O/alarm/blackbird.wav@auto"
want robin && $C "$OUT/robin.png" 10000 3.5 "Robin: real (2 songs) vs ours (2 renditions)" \
  "real=$R/robin/671058_Andrew.soundscape.wav@2.3" "real=$R/robin/671058_Andrew.soundscape.wav@19.9" "ours=$O/robin.wav@auto" "ours=$O/robin.wav@auto8"
want chaffinch && $C "$OUT/chaffinch.png" 10000 3.2 "Chaffinch: real (2 song types) vs ours (2 renditions)" \
  "real=$R/chaffinch/332851_Travesia_Sonora.wav@0.0" "real=$R/chaffinch/725328_Sacha.Julien.wav@9.9" "ours=$O/chaffinch.wav@auto" "ours=$O/chaffinch_2.wav@auto"
want great_tit && $C "$OUT/great_tit.png" 10000 4 "Great tit: real (3 song types) vs ours" \
  "real=$R/great_tit/418102_straget.wav@0.8" "real=$R/great_tit/389404_BeeProductive.wav@4.3" "real=$R/great_tit/346066_pulswelle.wav@1.3" "ours=$O/great_tit.wav@auto" "ours=$O/great_tit_2.wav@auto"
want great_tit && $C "$OUT/great_tit_alarm.png" 10000 3 "Great tit churr: real vs ours" \
  "real=$R/great_tit/418102_straget.wav@12.9" "ours=$O/alarm/great_tit.wav@auto"
want cuckoo && $C "$OUT/cuckoo.png" 1500 5 "Cuckoo: real (2 birds) vs ours" \
  "real=$R/cuckoo/688844_Elanor1995.wav@30" "real=$R/cuckoo/377213_jaromarsalek.wav@2" "ours=$O/cuckoo.wav@auto" "ours=$O/cuckoo_2.wav@auto"
want wood_pigeon && $C "$OUT/wood_pigeon.png" 2000 5 "Wood pigeon: real vs ours" \
  "real=$R/wood_pigeon/814890_Sadiquecat.wav@28.0" "ours=$O/wood_pigeon.wav@auto" "ours=$O/wood_pigeon_2.wav@auto"
want collared_dove && $C "$OUT/collared_dove.png" 2000 5 "Collared dove: real (2 birds) vs ours" \
  "real=$R/collared_dove/610563_randomthoughts7.wav@1.3" "real=$R/collared_dove/516091_Karola3206.wav@0" "ours=$O/collared_dove.wav@auto" "ours=$O/collared_dove_2.wav@auto"
want crow && $C "$OUT/crow.png" 8000 3.5 "Crow: real (2 birds) vs ours" \
  "real=$R/crow/181088_cfrooos.wav@11.0" "real=$R/crow/741366_Mish7913.wav@0" "ours=$O/crow.wav@auto" "ours=$O/crow_2.wav@auto"
want tawny_owl && $C "$OUT/tawny_owl.png" 2500 5 "Tawny owl: real (2 birds) vs ours (the hu + tremolo hoot)" \
  "real=$R/tawny_owl/745208_Patrick_Corra.wav@4.7" "real=$R/tawny_owl/745208_Patrick_Corra.wav@26.0" "ours=$O/tawny_owl.wav@auto4"
want tawny_owl && $C "$OUT/tawny_owl_kewick.png" 4000 3 "Tawny owl ke-wick: real vs ours" \
  "real=$R/tawny_owl/735744_Vrymaa.wav@4.2" "ours=$O/alarm/tawny_owl.wav@auto"
want house_sparrow && $C "$OUT/house_sparrow.png" 10000 3 "House sparrow: real (2 birds) vs ours" \
  "real=$R/house_sparrow/404663_straget.wav@14.5" "real=$R/house_sparrow/383160_BeeProductive.wav@5.5" "ours=$O/house_sparrow.wav@auto" "ours=$O/house_sparrow_2.wav@auto"
want herring_gull && $C "$OUT/herring_gull.png" 6000 4.2 "Herring gull: real (long call, mew) vs ours" \
  "real=$R/herring_gull/538016_Canardo55.wav@8.3" "real=$R/herring_gull/538015_Canardo55.wav@0" "ours=$O/herring_gull.wav@auto" "ours=$O/herring_gull_2.wav@auto"
want nightingale && $C "$OUT/nightingale.png" 10000 4 "Nightingale: real vs ours" \
  "real=$R/nightingale/521035_smand.wav@0.6" "real=$R/nightingale/521035_smand.wav@13.4" "ours=$O/nightingale.wav@auto" "ours=$O/nightingale.wav@auto10"
want skylark && $C "$OUT/skylark.png" 10000 4 "Skylark: real vs ours" \
  "real=$R/skylark/387426_Kinoton.wav@7.7" "real=$R/skylark/399221_Veridiansunrise.wav@4" "ours=$O/skylark.wav@auto" "ours=$O/skylark_2.wav@auto"
want woodpecker && $C "$OUT/woodpecker.png" 8000 3 "Great spotted woodpecker drumming: real vs ours" \
  "real=$R/woodpecker/428146_naturenotesuk.wav@2.2" "real=$R/woodpecker/867542_zachrau.wav@1.2" "ours=$O/woodpecker.wav@auto" "ours=$O/woodpecker_2.wav@auto"
exit 0
