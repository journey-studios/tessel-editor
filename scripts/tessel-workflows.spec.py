"""Regression checks for the command adapter without starting Chrome."""
import signal
import unittest
from unittest.mock import patch

import tessel_workflows as cli


class WorkflowsTests(unittest.TestCase):
    def test_camera_preview_keeps_native_preset_by_default(self):
        options = cli.parser(None).parse_args(["export", "project.json", "out.mp4", "--clip", "take"])
        with (
            patch.object(cli, "with_backend", side_effect=lambda _command, run: run(None)),
            patch.object(cli, "request"),
            patch.object(cli, "run_job") as job,
        ):
            cli.export(options)
        self.assertEqual(job.call_args.args[1], "export.start")
        self.assertEqual(job.call_args.kwargs["camera_preview"], "take")
        self.assertNotIn("preset", job.call_args.kwargs)
        self.assertNotIn("range", job.call_args.kwargs)

    def test_late_interrupt_keeps_the_already_published_result(self):
        def reply(_process, operation, **_fields):
            if operation == "export.start":
                signal.raise_signal(signal.SIGINT)
                return {"job": "video"}
            if operation == "job.cancel":
                raise cli.NativeError(operation, {"code": "job_finished"})
            self.assertEqual(operation, "job.get")
            return {"state": "succeeded", "stage": "done", "done": 1, "total": 1,
                    "result": {"frames": 4}}

        with (
            patch.object(cli, "request", side_effect=reply),
            patch.object(cli.subprocess, "Popen") as spawn,
        ):
            # A native JSONL session exits 1 when any request was rejected.
            spawn.return_value.__enter__.return_value.returncode = 1
            result = cli.with_backend([], lambda process: cli.run_job(process, "export.start"))
        self.assertEqual(result, {"frames": 4})

    def test_interrupted_job_waits_for_native_cancellation(self):
        with patch.object(cli, "request") as request:
            def reply(_process, operation, **_fields):
                if operation == "export.start":
                    signal.raise_signal(signal.SIGINT)
                    return {"job": "video"}
                if operation == "job.cancel":
                    return {}
                return {"state": "cancelled", "stage": "render", "done": 3, "total": 8}

            request.side_effect = reply
            with self.assertRaises(KeyboardInterrupt):
                cli.run_job(None, "export.start")
        self.assertEqual(
            [call.args[1] for call in request.call_args_list],
            ["export.start", "job.cancel", "job.get"],
        )


if __name__ == "__main__":
    unittest.main()
