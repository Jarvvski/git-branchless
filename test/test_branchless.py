import io

import py

from branchless.__main__ import main
from helpers import compare, git, git_commit_file, git_init_repo


def test_help() -> None:
    with io.StringIO() as out, io.StringIO() as err:
        assert main(["--help"], out=out, err=err) == 0
        assert "usage: branchless" in out.getvalue()

    with io.StringIO() as out, io.StringIO() as err:
        assert main([], out=out, err=err) == 1
        assert "usage: branchless" in out.getvalue()


def test_commands(tmpdir: py.path.local) -> None:
    with tmpdir.as_cwd():
        git_init_repo()
        git_commit_file(name="test", time=1)

        compare(
            actual=git("smartlog"),
            expected="""\
:
@ 3df4b935 (master) create test.txt
""",
        )

        compare(
            actual=git("hide", ["3df4b935"]),
            expected="""\
Hid commit: 3df4b935 create test.txt
To unhide this commit, run: git unhide 3df4b935
""",
        )

        compare(
            actual=git("unhide", ["3df4b935"]),
            expected="""\
Unhid commit: 3df4b935 create test.txt
To hide this commit, run: git hide 3df4b935
""",
        )

        compare(
            actual=git("prev"),
            expected="""\
branchless: git checkout HEAD^
@ f777ecc9 create initial.txt
|
O 3df4b935 (master) create test.txt
""",
        )

        compare(
            actual=git("next"),
            expected="""\
branchless: git checkout 3df4b9355b3b072aa6c50c6249bf32e289b3a661
:
@ 3df4b935 (master) create test.txt
""",
        )
