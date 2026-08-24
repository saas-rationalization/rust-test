// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {AssessmentToken} from "./AssessmentToken.sol";

contract ChainVault {
    AssessmentToken public immutable token;
    mapping(address => uint256) public deposits;

    event Deposited(address indexed account, uint256 amount);
    event Withdrawn(address indexed account, uint256 amount);

    constructor(address tokenAddress) {
        token = AssessmentToken(tokenAddress);
    }

    function deposit(uint256 amount) external {
        require(amount > 0, "amount=0");
        require(token.transferFrom(msg.sender, address(this), amount), "transfer failed");
        deposits[msg.sender] += amount;
        emit Deposited(msg.sender, amount);
    }

    function withdraw(uint256 amount) external {
        require(amount > 0, "amount=0");
        require(deposits[msg.sender] >= amount, "insufficient deposit");
        deposits[msg.sender] -= amount;
        require(token.transfer(msg.sender, amount), "transfer failed");
        emit Withdrawn(msg.sender, amount);
    }

    function balanceOf(address account) external view returns (uint256) {
        return deposits[account];
    }
}
