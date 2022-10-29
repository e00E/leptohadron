# Leptohadron

Terminal UI tool aiding in cleaning up installed Arch Linux packages.

Over time, systems tend to accumulate installed packages. It is tedious to find out why a package is installed by running commands to see dependants and dependencies for individual packages by hand. This tool makes this process less tedious by making the dependency tree easy to navigate.

# Usage

```
Key                    Action

left, right            move between lists
up, down, PgUp, PgDown move in list
1, 0                   move to start/end of list
Enter                  focus center list on selected entry
s                      toggle sorting between alphabetical-asc and size-desc in active view
e                      toggle showing only explicitly installed packages in main view
/                      start entering search term, enter to search, esc to cancel
n                      go to next search match downwards
N                      go to next search match upwards
?                      toggle help
q                      quit
```

The interface is divided into three lists of packages that are navigated with the arrow keys. The center list shows all installed packages. The currently selected package in the center list is called the main package.

Based on the main package the content of the side lists changes. The left list shows packages depending on the main package. The right list shows packages the main package depends on. Pressing enter on an entry in a side list makes that package the new main package.

After identifying and removing packages it can be useful clean up other obsolete packages (orphans) as explained in the [ArchWiki](
https://wiki.archlinux.org/title/Pacman/Tips_and_tricks#Removing_unused_packages_(orphans)).
