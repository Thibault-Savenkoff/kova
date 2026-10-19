// Runtime de Kova : le petit morceau de C collé à chaque exécutable (`kova build`).
// Les messages doivent être identiques à ceux de l'interpréteur (src/kir.rs, `texte_panne`).

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Fournis par le code produit par Cranelift (src/codegen.rs).
extern const char kova_fichier[];
void kova_main(void);

// `ch + 1` sur le dernier chiffre (chiffres 0-9), avec retenue. Renvoie 1 si l'exposant augmente.
static int plus_un(char *ch, int n) {
    for (int i = n - 1; i >= 0; i--) {
        if (ch[i] < 9) { ch[i]++; return 0; }
        ch[i] = 0;
    }
    ch[0] = 1; // 999 → 100
    return 1;
}

// Vrai si les chiffres `ch` (n chiffres, exposant e) redonnent exactement a.
static int relire(const char *ch, int n, int e, double a) {
    char buf[40];
    int k = 0;
    buf[k++] = '0' + ch[0];
    buf[k++] = '.';
    for (int i = 1; i < n; i++) buf[k++] = '0' + ch[i];
    snprintf(buf + k, sizeof buf - k, "e%d", e);
    return strtod(buf, NULL) == a;
}

// Les chiffres les plus courts de a (> 0) qui redonnent a : même calcul que
// `plus_courts` dans src/kir.rs. Renvoie le nombre de chiffres ; *e = exposant du premier.
static int plus_courts(double a, char *res, int *e) {
    static char buf[1000];
    snprintf(buf, sizeof buf, "%.800e", a); // tous les chiffres exacts (glibc)
    char exact[820];
    int n = 0;
    char *c = buf;
    for (; *c != 'e'; c++)
        if (*c != '.') exact[n++] = *c - '0';
    *e = atoi(c + 1);
    while (n > 1 && exact[n - 1] == 0) n--;

    for (int p = 1; p <= 17; p++) {
        if (n <= p) { memcpy(res, exact, n); return n; }
        // Les deux nombres de p chiffres qui entourent a : `bas` (coupé) et `haut`.
        char bas[17], haut[17];
        memcpy(bas, exact, p);
        memcpy(haut, exact, p);
        int e_haut = *e + plus_un(haut, p);
        int apres = 0;
        for (int i = p + 1; i < n; i++) apres |= exact[i];
        int monter = exact[p] > 5 || (exact[p] == 5 && (apres || bas[p - 1] % 2 == 1));
        char *proche = monter ? haut : bas, *loin = monter ? bas : haut;
        int e_proche = monter ? e_haut : *e, e_loin = monter ? *e : e_haut;
        char *gagnant = NULL;
        int e_gagnant = 0;
        if (relire(proche, p, e_proche, a)) { gagnant = proche; e_gagnant = e_proche; }
        else if (relire(loin, p, e_loin, a)) { gagnant = loin; e_gagnant = e_loin; }
        if (gagnant) {
            while (p > 1 && gagnant[p - 1] == 0) p--;
            memcpy(res, gagnant, p);
            *e = e_gagnant;
            return p;
        }
    }
    abort(); // 17 chiffres suffisent toujours
}

// Un Float comme `texte_float` dans src/kir.rs : `3.0`, `0.30000000000000004`, `1e16`, `NaN`.
static void ecrire_float(FILE *f, double x) {
    if (isnan(x)) { fputs("NaN", f); return; } // printf écrirait `-nan` selon le signe
    if (isinf(x)) { fputs(x > 0 ? "inf" : "-inf", f); return; }
    if (signbit(x)) fputc('-', f); // garde le signe de -0.0
    double a = fabs(x);
    char ch[17];
    int exp = 0, n = 1;
    if (a == 0.0) ch[0] = 0;
    else n = plus_courts(a, ch, &exp);
    for (int i = 0; i < n; i++) ch[i] += '0';

    if (a == 0.0 || (a >= 1e-4 && a < 1e16)) {
        if (exp >= 0) {
            for (int i = 0; i <= exp; i++) fputc(i < n ? ch[i] : '0', f);
            fputc('.', f);
            if (n > exp + 1) fwrite(ch + exp + 1, 1, n - exp - 1, f);
            else fputc('0', f);
        } else {
            fputs("0.", f);
            for (int i = 0; i < -exp - 1; i++) fputc('0', f);
            fwrite(ch, 1, n, f);
        }
    } else {
        fputc(ch[0], f);
        if (n > 1) { fputc('.', f); fwrite(ch + 1, 1, n - 1, f); }
        fprintf(f, "e%d", exp);
    }
}

// Un Char (point Unicode) en UTF-8.
static void ecrire_char(FILE *f, uint32_t c) {
    if (c < 0x80) {
        fputc(c, f);
    } else if (c < 0x800) {
        fputc(0xC0 | c >> 6, f);
        fputc(0x80 | (c & 0x3F), f);
    } else if (c < 0x10000) {
        fputc(0xE0 | c >> 12, f);
        fputc(0x80 | (c >> 6 & 0x3F), f);
        fputc(0x80 | (c & 0x3F), f);
    } else {
        fputc(0xF0 | c >> 18, f);
        fputc(0x80 | (c >> 12 & 0x3F), f);
        fputc(0x80 | (c >> 6 & 0x3F), f);
        fputc(0x80 | (c & 0x3F), f);
    }
}

// Même ordre que `Affichage` dans src/kir.rs.
enum { INT, BOOL, FLOAT, CHAR };

static void ecrire(FILE *f, int64_t valeur, int64_t genre) {
    double x;
    switch (genre) {
    case INT: fprintf(f, "%lld", (long long)valeur); break;
    case BOOL: fputs(valeur ? "true" : "false", f); break;
    case FLOAT: memcpy(&x, &valeur, sizeof x); ecrire_float(f, x); break;
    case CHAR: ecrire_char(f, (uint32_t)valeur); break;
    }
}

void kova_afficher(int64_t valeur, int64_t genre, int64_t ligne) {
    ecrire(stdout, valeur, genre);
    if (ligne) putchar('\n');
}

// Même ordre que `Panne` dans src/kir.rs.
static const char *const PANNES[] = {
    "dépassement d'entier",
    "division par zéro",
    "conversion impossible : ce Float ne tient pas dans un Int",
};

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
