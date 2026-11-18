# SPDX-License-Identifier: GPL-3.0-or-later
# POST_BUILD step of fefw.exe (CMakeLists.txt): copies the site list the libraries were lifted with (SRC, <lib dir>/main.sites) next
# to the exe as DST (fefw.sites). Run at build time, not configure time, so a recomp build made after configuring is picked up. With
# no list, a stale fefw.sites is removed: the runtime then skips its hook checks instead of checking against the wrong build.
if(EXISTS "${SRC}")
    file(COPY_FILE "${SRC}" "${DST}" ONLY_IF_DIFFERENT)
else()
    file(REMOVE "${DST}")
    message(WARNING "${SRC} is missing (rebuild the libraries with recomp/scripts/build_game.cmd fefw): no fefw.sites, so hooks and mods' sites are not checked")
endif()
