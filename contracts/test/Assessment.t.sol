// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {AssessmentToken} from "../src/AssessmentToken.sol";
import {ChainVault} from "../src/ChainVault.sol";

contract AssessmentTest is Test {
    AssessmentToken internal token;
    ChainVault internal vault;

    function setUp() public {
        token = new AssessmentToken();
        vault = new ChainVault(address(token));
        token.approve(address(vault), type(uint256).max);
    }

    function testDepositAndWithdraw() public {
        token.transfer(address(this), 100 ether);
        token.approve(address(vault), 100 ether);

        vault.deposit(40 ether);
        assertEq(vault.balanceOf(address(this)), 40 ether);

        vault.withdraw(15 ether);
        assertEq(vault.balanceOf(address(this)), 25 ether);
    }
}
