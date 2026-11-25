/*
 * ps3recomp - loose-file game mods (included once, by ppu_fs.cpp)
 *
 * The mods made for the Yakuza titles (Ryu ga Gotoku Kenzan!, Yakuza: Dead Souls) are plain
 * replacement files with a note saying where they go ("Replace sh_iori_ps3.par in ./bin/shader",
 * "This mod replaces ./bin/scenario/mail.bin"). On a console or RPCS3 the player copies them over
 * the game's own. Here nothing in the game folder is written: the files stay in a mods folder and
 * a read of the game file gets the mod's instead.
 *
 *   PS3_LOOSE_MODS_DIR=<dir>   the mods folder; unset = no mods.
 *     <dir>/<mod>/...          one folder per mod, applied in name order (case-insensitive);
 *                              a folder whose name starts with '_' or '.' is turned off.
 *     a file at <mod>/<path>   replaces USRDIR/<path> when the game has that file
 *                              (bin/shader/sh_iori_ps3.par); a "PS3_GAME/USRDIR/" or "USRDIR/"
 *                              prefix is allowed. Otherwise, when its NAME exists exactly once
 *                              in the game's USRDIR, it replaces that file (a mod zip that has
 *                              the file alone at its top level, as both Kenzan mods do).
 *     *.txt and *.md files are the mods' notes, never game files.
 *   PS3_LOOSE_MODS=0           use none of them.
 *
 * Applies to every read-only open and stat of /dev_bdvd/PS3_GAME/USRDIR/..., /app_home/... and
 * the HDD copy /dev_hdd0/game/<title>/USRDIR/... (Dead Souls installs its data there and reads
 * that copy). A file two mods change comes from the first one; the clash is logged.
 * Log lines start "[mods]".
 */
#ifndef PPU_FS_LOOSE_MODS_H
#define PPU_FS_LOOSE_MODS_H

#include <algorithm>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

namespace loosemods {

static std::unordered_map<std::string, std::string> s_map;   /* usrdir-relative, lower case -> host file */
static std::once_flag s_once;
static bool s_on = false;

static std::string lower(std::string s) { for (char& c : s) c = (char)tolower((unsigned char)c); return s; }
static bool ends_i(const std::string& s, const char* suf)
{
    const size_t n = strlen(suf);
    return s.size() >= n && lower(s.substr(s.size() - n)) == suf;
}

/* every regular file below `dir`, as paths relative to it ('/' separated) */
static void walk(const std::string& dir, const std::string& rel, std::vector<std::string>& out, int depth)
{
    if (depth > 24) return;
    DIR* d = opendir(dir.c_str());
    if (!d) return;
    while (struct dirent* e = readdir(d)) {
        const char* n = e->d_name;
        if (!strcmp(n, ".") || !strcmp(n, "..")) continue;
        const std::string full = dir + "/" + n, r = rel.empty() ? std::string(n) : rel + "/" + n;
        struct stat st;
        if (stat(full.c_str(), &st) != 0) continue;
        if (st.st_mode & S_IFDIR) walk(full, r, out, depth + 1);
        else out.push_back(r);
    }
    closedir(d);
}

static void init()
{
    const char* dir = getenv("PS3_LOOSE_MODS_DIR");
    const char* use = getenv("PS3_LOOSE_MODS");
    if (!dir || !*dir || (use && *use == '0')) return;

    /* the game's USRDIR: every file, by relative path and by name */
    std::string usr = std::string(ppu_vfs_root) + "/PS3_GAME/USRDIR";
    std::vector<std::string> game;
    walk(usr, "", game, 0);
    if (game.empty()) { usr = ppu_vfs_root; walk(usr, "", game, 0); }   /* a root that is USRDIR itself */
    std::unordered_map<std::string, std::string> by_rel;               /* lower rel -> real rel */
    std::unordered_map<std::string, std::vector<std::string>> by_name; /* lower name -> lower rels */
    for (const std::string& r : game) {
        const std::string lr = lower(r);
        by_rel[lr] = r;
        by_name[lr.substr(lr.rfind('/') + 1)].push_back(lr);
    }

    std::vector<std::string> mods;
    if (DIR* d = opendir(dir)) {
        while (struct dirent* e = readdir(d)) {
            const std::string n = e->d_name;
            if (n.empty() || n[0] == '.' || n[0] == '_') continue;
            struct stat st;
            if (stat((std::string(dir) + "/" + n).c_str(), &st) == 0 && (st.st_mode & S_IFDIR)) mods.push_back(n);
        }
        closedir(d);
    }
    std::sort(mods.begin(), mods.end(), [](const std::string& a, const std::string& b) { return lower(a) < lower(b); });

    for (const std::string& m : mods) {
        std::vector<std::string> files;
        walk(std::string(dir) + "/" + m, "", files, 0);
        int used = 0;
        for (const std::string& f : files) {
            if (ends_i(f, ".txt") || ends_i(f, ".md")) continue;
            std::string lr = lower(f);
            for (const char* pre : { "ps3_game/usrdir/", "usrdir/" })
                if (lr.compare(0, strlen(pre), pre) == 0) { lr = lr.substr(strlen(pre)); break; }
            std::string target;
            if (by_rel.count(lr)) target = lr;
            else {
                auto it = by_name.find(lr.substr(lr.rfind('/') + 1));
                if (it != by_name.end() && it->second.size() == 1) target = it->second[0];
                else if (it != by_name.end())
                    fprintf(stderr, "[mods] %s: %s -- %zu game files have that name; put it in its folder "
                                    "(e.g. %s)\n", m.c_str(), f.c_str(), it->second.size(), by_rel[it->second[0]].c_str());
                else fprintf(stderr, "[mods] %s: %s -- the game has no file of that name, left out\n", m.c_str(), f.c_str());
            }
            if (target.empty()) continue;
            const std::string host = std::string(dir) + "/" + m + "/" + f;
            auto ins = s_map.emplace(target, host);
            if (!ins.second) {
                fprintf(stderr, "[mods] %s: %s is already changed by an earlier mod (%s), left out\n",
                        m.c_str(), by_rel[target].c_str(), ins.first->second.c_str());
                continue;
            }
            fprintf(stderr, "[mods] %s: %s replaces USRDIR/%s\n", m.c_str(), f.c_str(), by_rel[target].c_str());
            used++;
        }
        fprintf(stderr, "[mods] using \"%s\": %d file(s)\n", m.c_str(), used);
    }
    s_on = !s_map.empty();
}

/* `guest` -> the USRDIR-relative path, lower case; empty when it is not a game data path */
static std::string usrdir_rel(const char* guest)
{
    std::string g = lower(guest);
    for (char& c : g) if (c == '\\') c = '/';
    const char* pfx[] = { "/dev_bdvd/ps3_game/usrdir/", "/app_home/" };
    for (const char* p : pfx)
        if (g.compare(0, strlen(p), p) == 0) return g.substr(strlen(p));
    if (g.compare(0, 15, "/dev_hdd0/game/") == 0) {           /* /dev_hdd0/game/<title>/usrdir/... */
        const size_t s = g.find('/', 15);
        if (s != std::string::npos && g.compare(s, 8, "/usrdir/") == 0) return g.substr(s + 8);
    }
    return std::string();
}

static void redirect(const char* guest, char* out, size_t cap)
{
    std::call_once(s_once, init);
    if (!s_on || !guest) return;
    const std::string rel = usrdir_rel(guest);
    if (rel.empty()) return;
    auto it = s_map.find(rel);
    if (it == s_map.end()) return;
    snprintf(out, cap, "%s", it->second.c_str());
    static std::atomic<int> n{0};
    if (n++ < 64) fprintf(stderr, "[mods] %s -> %s\n", guest, out);
}

} // namespace loosemods

#endif
