#!/usr/bin/python3
"""统一执行按职责拆分的虚构发行契约，不访问生产。"""
import unittest
from activation_cases.records import RecordsTests
from activation_cases.candidate import CandidateTests
from activation_cases.prepare import PrepareTests
from activation_cases.binding import BindingTests
from activation_cases.deployment import DeploymentTests
from activation_cases.operations import OperationTests

if __name__ == '__main__':
    unittest.main()
