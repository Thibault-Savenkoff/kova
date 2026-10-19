// Runtime de Kova : le petit morceau de C collé à chaque exécutable (`kova build`).
// Les messages doivent être identiques à ceux de l'interpréteur (src/kir.rs, `texte_panne`).

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

// Fournis par le code produit par Cranelift (src/codegen.rs).
extern const char kova_fichier[];
void kova_main(void);

void kova_afficher(int64_t valeur, int64_t booleen, int64_t ligne) {
    if (booleen) fputs(valeur ? "true" : "false", stdout);
    else printf("%lld", (long long)valeur);
    if (ligne) putchar('\n');
}

// Même ordre que `Panne` dans src/kir.rs.
static const char *const PANNES[] = {"dépassement d'entier", "division par zéro"};

void kova_panne(int64_t genre, int64_t ligne, int64_t colonne) {
    fflush(stdout);
    fprintf(stderr, "erreur: %s\n  --> %s:%lld:%lld\n", PANNES[genre], kova_fichier, (long long)ligne,
            (long long)colonne);
    exit(101);
}

int main(void) {
    kova_main();
    return 0;
}
